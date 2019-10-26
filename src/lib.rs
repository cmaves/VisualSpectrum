
pub mod audio {
    use jack;
    use std::sync::mpsc;
    use std::mem;
    use rustfft::algorithm::{Radix4};
    use rustfft::FFT;
    use rustfft::num_complex::Complex;
    use rustfft::num_traits::Zero;

    pub struct ActiveProducer {
        sample_size: usize,
        recv: mpsc::Receiver<StereoSample>,
        source: ActiveSource
    }
    pub struct PendingProducer {
        sample_size: usize,
        sender: mpsc::SyncSender<StereoSample>,
        recv: mpsc::Receiver<StereoSample>,
        source: InactiveSource
    }
    enum InactiveSource {
       Jack(jack::Client)
    }
    enum ActiveSource {
        Jack(jack::AsyncClient<Notifications, FrameHandler>)
    }
    
    impl PendingProducer {
        pub fn new_jack(sample_size: usize) -> Result<Self, SpectrumError> {
            let (client, _status) = match jack::Client::new("spectrum", jack::ClientOptions::NO_START_SERVER) {
                Ok(ok) => ok,
                Err(err) => return Err(SpectrumError::JackError(err))
            };
            let (sender, recv) = mpsc::sync_channel(client.sample_rate() / sample_size);
            Ok(PendingProducer {
                sample_size: sample_size,
                sender: sender,
                recv: recv,
                source: InactiveSource::Jack(client),
            })
        }
        pub fn activate(self) -> Result<ActiveProducer, SpectrumError>  {
            match self.source {
                InactiveSource::Jack(s) => {
                    let in_l = match s.register_port("left", jack::AudioIn::default()) {
                        Ok(ok) => ok,
                        Err(err) => return Err(SpectrumError::JackError(err))
                    };
                    let in_r = match s.register_port("right", jack::AudioIn::default()) {
                        Ok(ok) => ok,
                        Err(err) => return Err(SpectrumError::JackError(err))
                    };
                    let handler = FrameHandler {
                        sample: StereoSample::new(self.sample_size),
                        sender: self.sender,
                        left: in_l,
                        right: in_r,
                        sample_size: self.sample_size
                    };
                    match s.activate_async(Notifications, handler) {
                        Ok(active) => {
                            Ok(ActiveProducer {
                                sample_size: self.sample_size,
                                recv: self.recv,
                                source: ActiveSource::Jack(active)
                            })
                        },
                        Err(err) => Err(SpectrumError::JackError(err))
                    }
                }
            }
        }
    }
    
    #[derive(Debug)]
    pub struct StereoSample {
        sample_size: usize,
        left: Vec<f32>,
        right: Vec<f32>

    }
    impl StereoSample {
        pub fn new(sample_size: usize) -> Self {
            StereoSample {
                sample_size: sample_size,
                left: Vec::with_capacity(sample_size),
                right: Vec::with_capacity(sample_size)
            }
        }
        pub fn extend(&mut self, left: &[f32], right: &[f32]) -> bool {
            let len = left.len();
            if len != right.len() {
                panic!("Error mismatched size");
            }
            let remaining = self.sample_size - self.left.len();
            if remaining > len {
                self.left.extend_from_slice(left);
                self.right.extend_from_slice(right);
                false  //return false if not full
            } else {
                self.left.extend_from_slice(&left[0..remaining]);
                self.right.extend_from_slice(&right[0..remaining]);
                true  //return true if full
            }
            
        }
        pub fn len(&self) -> usize {
            let l_len = self.left.len();
            let r_len = self.right.len();
            if l_len > r_len {
                l_len
            } else {
                r_len
            }
        }
        
        pub fn spectrogram(&self, fft: &Radix4<f32>) -> (Vec<f32>, Vec<f32>) {
            let mut l_in: Vec<Complex<f32>> = self.left.iter().map(|f| Complex::new(*f,0.0)).collect();
            let mut r_in: Vec<Complex<f32>> = self.right.iter().map(|f| Complex::new(*f,0.0)).collect();
            let mut l_out: Vec<Complex<f32>> = vec![Complex::zero(); self.sample_size];
            let mut r_out: Vec<Complex<f32>> = vec![Complex::zero(); self.sample_size];
            // proectss FFT of the two channels
            fft.process_multi(&mut l_in, &mut l_out);
            fft.process_multi(&mut r_in, &mut r_out);

            /* normalize complex-valued amp and convert to amp-to-dB log_10 (amp^2).
                Using norm_sqr() is a simplification that 
                allows us to avoid an expensive sqrt operation for a value
                we would either just sqaure before being input to log10() ( or double after the log10(0).
            */
            (l_out.into_iter().map(|c| c.norm_sqr().log10()*10.0).collect(),
            r_out.into_iter().map(|c| c.norm_sqr().log10()*10.0).collect())
        }
   
    }
    #[derive(Debug)]
    pub enum SpectrumError {
        JackError(jack::Error),
        SpectrumError(&'static str)
    }
    struct FrameHandler {
        sample: StereoSample,
        sender: mpsc::SyncSender<StereoSample>,
        left: jack::Port<jack::AudioIn>,
        right: jack::Port<jack::AudioIn>,
        sample_size: usize
    }
    impl jack::ProcessHandler for FrameHandler {
        fn process(&mut self, _: &jack::Client, ps: &jack::ProcessScope) -> jack::Control {
            let in_l_sample = self.left.as_slice(ps);
            let in_r_sample = self.right.as_slice(ps);
            if self.sample.extend(in_l_sample, in_r_sample) {
                /* TODO: evaluate potential performance gains by eliminating allocation, that will
                        be dropped every 8 audio frames. This is probably expensive and could in
                        theory be replace by either some kind of mutex protected circular buffer or
                        perhaps passing allocated Vec in a return mpsc-channel.
                        Profiling needs to be done to determine where this thread is actually spending
                        time. It could be that this is just a neccassaryly expensive component.
                */
                let mut ss = StereoSample::new(self.sample_size);
                mem::swap(&mut ss, &mut self.sample); // in theory this should run without issue
                self.sender.send(ss).unwrap();
            }
            jack::Control::Continue
        }
    }
    impl Iterator for ActiveProducer {
        type Item = StereoSample;
        fn next(&mut self) -> Option<Self::Item> {
            match self.recv.recv() {
                Ok(ss) => Some(ss),
                Err(_) => None
            }
        }
    }
    
    struct Notifications;
    
    impl jack::NotificationHandler for Notifications {
        fn xrun(&mut self, _: &jack::Client) -> jack::Control {
            eprintln!("xrun occurred");
            jack::Control::Continue
        }
    }
}


pub mod led {
    use rs_ws281x;
    use crate::audio;
    use rustfft::algorithm::Radix4;
    use std::sync::mpsc;
    use std::thread;
    //R color order
    const COLORS: [[u8; 4]; 4] = [[0, 0, 255, 0],
                    [0, 185,185,0],
                    [0, 255, 0, 0],
                    [255, 0, 0, 0]];
    // Approximate ITU-R 468 weightings
    const WEIGHT: [f32; 256] = [0.0,-20.45,-14.43,-10.92,-8.43,-6.50,-4.93,-3.61,
        -2.47,-1.47,-0.58,0.22,0.95,1.61,2.22,2.79,3.31,3.80,4.26,4.68,5.09,
        5.47,5.83,6.17,6.49,6.80,7.09,7.37,7.64,7.89,8.14,
        8.37,8.60,8.81,9.02,9.22,9.41,9.59,9.77,9.93,10.10,
        10.25,10.40,10.54,10.68,10.81,10.94,11.06,11.17,11.27,11.38,
        11.47,11.56,11.64,11.72,11.79,11.85,11.91,11.97,12.01,12.05,
        12.09,12.12,12.14,12.16,12.17,12.18,12.18,12.17,12.16,12.15,
        12.13,12.10,12.07,12.04,12.00,11.95,11.91,11.85,11.79,11.73,
        11.67,11.60,11.52,11.44,11.36,11.27,11.18,11.08,10.98,10.87,
        10.76,10.64,10.51,10.38,10.24,10.10,9.95,9.79,9.63,9.45,
        9.27,9.08,8.89,8.68,8.47,8.25,8.02,7.78,7.54,7.28,
        7.02,6.75,6.48,6.20,5.91,5.61,5.31,5.01,4.70,4.38,
        4.06,3.74,3.41,3.09,2.76,2.42,2.09,1.75,1.41,1.08,
        0.74,0.40,0.06,-0.28,-0.62,-0.96,-1.29,-1.63,-1.97,-2.30,
        -2.63,-2.97,-3.30,-3.63,-3.95,-4.28,-4.60,-4.93,-5.25,-5.57,
        -5.88,-6.20,-6.51,-6.83,-7.13,-7.44,-7.75,-8.05,-8.35,-8.65,
        -8.95,-9.25,-9.54,-9.84,-10.13,-10.42,-10.70,-10.99,-11.27,-11.55,
        -11.83,-12.11,-12.38,-12.66,-12.93,-13.20,-13.47,-13.74,-14.00,-14.27,
        -14.53,-14.79,-15.05,-15.31,-15.56,-15.82,-16.07,-16.32,-16.57,-16.82,
        -17.06,-17.31,-17.55,-17.79,-18.04,-18.27,-18.51,-18.75,-18.98,-19.22,
        -19.45,-19.68,-19.91,-20.14,-20.37,-20.59,-20.82,-21.04,-21.27,-21.49,
        -21.71,-21.93,-22.14,-22.36,-22.58,-22.79,-23.00,-23.21,-23.43,-23.64,
        -23.84,-24.05,-24.26,-24.46,-24.67,-24.87,-25.08,-25.28,-25.48,-25.68,
        -25.88,-26.07,-26.27,-26.47,-26.66,-26.86,-27.05,-27.24,-27.43,-27.62,
        -27.81,-28.00,-28.19,-28.38,-28.56,-28.75,-28.93,-29.12,-29.30,-29.48,
        -29.66,-29.84,-30.02,-30.20,-30.38];

    pub struct Controller {
       controller: rs_ws281x::Controller,
       invert: bool,
       colors: [[u8; 4]; 4],
       alg: Algorithm
    }

    impl Controller {
        pub fn new(pin: i32, count: i32, invert: bool, brightness: f32) -> Controller {
            if brightness > 1.0 || brightness <= 0.0 {
                panic!("brightness must be (0,1]");
            }
            let channel = rs_ws281x::ChannelBuilder::new()
                .pin(pin).strip_type(rs_ws281x::StripType::Ws2812)
                .count(count)
                .brightness(255).build();
            let controller = rs_ws281x::ControllerBuilder::new()
                .freq(800_000)
                .channel(0, channel).build().unwrap();
            let mut colors = [[0; 4]; 4];
            for i in 0..4 {
                for j in 0..4 {
                    colors[i][j] = ((COLORS[i][j] as f32) * brightness) as u8;
                }
            }
            Controller {
                controller: controller,
                invert: invert,
                colors: colors,
                alg: Algorithm::Linear
            }
        }
        pub fn set_alg(mut self, alg: Algorithm) -> Self {
            self.alg = alg;
            self
        }
        pub fn display(mut self, source: audio::PendingProducer) -> Self {
            let mut active = source.activate().unwrap(); 
            let (sender, recv) = mpsc::sync_channel(1);
            let led_num = self.controller.leds(0).len();
            let invert = self.invert;
            let colors = self.colors;
            let alg = self.alg;
            thread::spawn(move || {
                let mut planner = Radix4::new(256, false);
                loop {
                    let ss = match active.next() {
                        Some(s) => s,
                        None => break
                    };
                    // fatch spectrogram using ffft
                    let (left, right) = ss.spectrogram(&mut planner);
                    if let Err(e) = sender.try_send(compute_4_bins(left,right,led_num,invert,&colors,alg)) {
                        match e {
                            mpsc::TrySendError::Full(_) => eprintln!("led render taking to long"),
                            mpsc::TrySendError::Disconnected(_) => break
                        }
                    };
                }
            });
            loop {
                let vals  = match recv.recv() {
                    Ok(o) => o,
                    Err(_) => break
                };
                let leds = self.controller.leds_mut(0); 
                // set using the half average method
                for i in 0..leds.len() {
                    for j in 0..3 { // currently the fourth value is never used so skip it
                        leds[i][j] = (((leds[i][j] as u16) * 2 + (vals[i][j] as u16) * 1) / 3) as u8;
                    }
                }
                //leds.copy_from_slice(&[[255,255,255,0];300]);
                //println!("{:?}\n\n\n\n", leds);
                self.controller.render();
            }
            self 
        }
    }
    #[derive(Copy,Clone)]
    pub enum Algorithm {
        Linear,
        Quadratic
    }
    fn compute_4_bins(left: Vec<f32>, right: Vec<f32>, leds: usize, invert: bool, 
                        colors: &[[u8; 4]; 4], alg: Algorithm) -> Vec<[u8; 4]> 
    {
        let n_windows = left.len() / 256;
        let n_win = left.len() / 256;
        // average channels
        let mut l_avg = [0.0; 256];
        let mut r_avg = [0.0; 256];
        for i in 0..256 {
            let mut l_sum = 0.0;
            let mut r_sum = 0.0;
            for n in 0..n_windows {
                l_sum += left[i+n*256];
                r_sum += right[i+n*256];
            }
            // average and apply weightins
            l_avg[i] = (l_sum / n_win as f32) + WEIGHT[i];
            r_avg[i] = (r_sum / n_win as f32) + WEIGHT[i];
        }

        let mut l_bins = [0.0; 4];
        l_bins[0] = max(&l_avg[1..3]); // Subwoofer
        l_bins[1] = max(&l_avg[3..6]); // Woofer
        l_bins[2] = max(&l_avg[6..21]); // Midrange
        l_bins[3] = max(&l_avg[21..256]); // Tweeter

        let mut r_bins = [0.0; 4];
        r_bins[0] = max(&r_avg[1..3]);
        r_bins[1] = max(&r_avg[3..6]);
        r_bins[2] = max(&r_avg[6..21]);
        r_bins[3] = max(&r_avg[21..256]);

        // scale to range of 0-100 floating point number
        match alg { 
            Linear => { 
                for i in 0..4 {
                    l_bins[i] = (l_bins[i]+ 40.0) * 2.000;
                    r_bins[i] = (r_bins[i] + 40.0) * 2.000;
                    if l_bins[i] > 100.0 {
                        l_bins[i] = 100.0;
                    } 
                    if r_bins[i] > 100.0 {
                        r_bins[i] = 100.0;
                    } 
                }
            },
            Quadratic => {
                for i in 0..4 {
                    l_bins[i] = (l_bins[i]+ 40.0) / 5.000;
                    l_bins[i] *= l_bins[i]; // square it 
                    r_bins[i] = (r_bins[i] + 40.0) / 5.000;
                    r_bins[i] *= r_bins[i]; // square it 
                    if l_bins[i] > 100.0 {
                        l_bins[i] = 100.0;
                    } 
                    if r_bins[i] > 100.0 {
                        r_bins[i] = 100.0;
                    } 
                }
            }
        }
        println!("{:?} {:?}", l_bins, r_bins);

        let part_len = leds / 8; // length of each bin representing a color
        let ratio = part_len as f32 / 100.0; // the ratio between the 0-100 range and the leds it
                                             // should effect
        let mut ret = vec![[0,0,0,0]; leds];
        // next section applies color values to left channel
        let bins = if invert { // invert the value if specified
            r_bins
        } else {
            l_bins
        };

        let mut l_sum: usize = 0;
        for i in 0..4 {
            let mut color_seg = (bins[i] * ratio).round() as usize;
            if color_seg < 1 {
                color_seg = 1;
            }
            for j in 0..color_seg {
                ret[l_sum + j] = colors[i];
            } 
            l_sum += color_seg;
        }

        // now repeat above for right channel
        let bins = if invert { // invert the value if specified
            l_bins
        } else {
            r_bins
        };

        let mut r_sum: usize = 0;
        let start = leds - 1;
        for i in 0..4 {
            let mut color_seg = (bins[i] * ratio).round() as usize;
            if color_seg < 1 {
                color_seg = 1;
            }
            for j in 0..color_seg {
                ret[start - r_sum - j] = colors[i];
            } 
            r_sum += color_seg;
        }
        //println!("{:?}\n\n\n\n", ret);
        ret
    }
    fn max(s: &[f32]) -> f32 {
        let mut ret = std::f32::NEG_INFINITY;
        for i in s {
            if *i > ret { ret = *i }
        } 
        ret
    }
    
}

use rs_ws281x;
use crate::audio;
use rustfft::algorithm::Radix4;
use std::sync::mpsc;
use std::thread;
use random_color::RandomColor;

//R color order
const COLORS: [[u8; 4]; 4] = [[0, 0, 255, 0],
                [0, 128,128,0],
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
   alg: Algorithm,
   pulse: bool,
   brightness: f32,
}

impl Controller {
    pub fn new(pin: i32, count: i32, invert: bool, brightness: f32, pulse: bool) -> Self {
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
            alg: Algorithm::Linear,
            pulse: pulse,
            brightness: brightness
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
        let pulse = self.pulse;
        let brightness = self.brightness;
        thread::spawn(move || {
            let mut planner = Radix4::new(256, false);
            let mut start_time = 0;
            let mut pc = PulseCalc::new(brightness);
            loop {
                let ss = match active.next() {
                    Some(s) => s,
                    None => break
                };
                // fatch spectrogram using ffft
                let res = if pulse && ss.is_no_sig() {
                    //eprintln!("pulse and no sig are true");
                    start_time += 1;
                    if start_time % 512 == 0 { pc.new_color(); }
                    if start_time >= 512 {
                        sender.try_send(vec![pc.ins_color(start_time,  512); led_num])
                    } else {
                        let (left, right) = ss.spectrogram(&mut planner);
                        sender.try_send(compute_4_bins(left,right,led_num,invert,&colors,alg))
                    }
                } else {
                    let (left, right) = ss.spectrogram(&mut planner);
                    start_time = 0;
                    sender.try_send(compute_4_bins(left,right,led_num,invert,
                        &colors,alg))
                };
                if let Err(e) = res {
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
                    leds[i][j] = (((leds[i][j] as u16) * 3 + (vals[i][j] as u16) * 1) / 4) as u8;
                }
            }
            //leds.copy_from_slice(&[[255,255,255,0];300]);
            //println!("{:?}\n\n\n\n", leds);
            self.controller.render();
        }
        self 
    }
}
#[derive(Copy,Clone,Debug)]
pub enum Algorithm {
    Linear,
    Quadratic
}
fn compute_4_bins(left: Vec<f32>, right: Vec<f32>, leds: usize, invert: bool, 
                    colors: &[[u8; 4]; 4], alg: Algorithm)
                    -> Vec<[u8; 4]> 
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

    // scale to range of (-inf, 100] floating point number
    match alg { 
        Algorithm::Linear => { 
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
        Algorithm::Quadratic => {
            for i in 0..4 {
                l_bins[i] = (l_bins[i]+ 40.0) / 5.000;
                // only square if  over zero because negative will become positive otherwise
                 if l_bins[i] > 0.0 { 
                    l_bins[i] *= l_bins[i]; // square it 
                }
                r_bins[i] = (r_bins[i] + 40.0) / 5.000;
                if r_bins[i] > 0.0 {
                    r_bins[i] *= r_bins[i]; // square it 
                }
                if l_bins[i] > 100.0 {
                    l_bins[i] = 100.0;
                } 
                if r_bins[i] > 100.0 {
                    r_bins[i] = 100.0;
                } 
            }
        }
    }
    //println!("{:?} {:?}", l_bins, r_bins);

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
pub struct PulseCalc {
    color: [u8; 4],
    rc: RandomColor,
    weight: f32
}
impl PulseCalc {
    pub fn new(weight: f32) -> Self {
        let mut ret = PulseCalc {
            color: [0; 4],
            rc: RandomColor::new(),
            weight: weight
        };
        ret.new_color();
        ret
    }
    pub fn new_color(&mut self) {
        /* 
            Get three rand f32 with range [0,255) with exponential distribution (or maybe log 
            I'm not sure about the name)
        */
        let rand = self.rc.to_rgb_array();
        for i in 0..3 {
            self.color[i] = rand[i] as u8;
        }
        eprintln!("{:?}", self.color);
    }
    pub fn ins_color(&self, time: usize, period: usize) -> [u8; 4] {
        let half = period / 2;
        let cap = time % period;
        let a_diff = (half as isize - cap as isize).abs() as usize; // absoulute difference
        let weight = (half - a_diff) as f32; // distance from max brightness
        let weight = linear_to_log(weight / (half as f32)) * self.weight;
        [(self.color[0] as f32  * weight) as u8, (self.color[1] as f32  * weight) as u8, (self.color[2] as f32 * weight) as u8, 0]
    }
}
fn linear_to_log(input: f32) -> f32 {
    2_f32.powf(input * 5.0 - 2.5) / 5.6568542
}
pub fn calc_nths(rands: [f32; 3]) -> [f32; 4] {
    #[cfg(test)]
    println!("input: {:?}",rands);
    let mut max = 0.0;
    for i in 0..3 {
        if max < rands[i] { max = rands[i]; }
    }
    
    let mut guess; // TODO: invistigate if there is better value or heuristic for this 
    if max >= 32.0 {
        guess = 1.59887068737177;
    } else if max >= 16.0 {
        guess = 1.99858835921471;
    } else if max >=  8.0 {
        guess = 2.66478447895295;
    } else if max >= 4.0 {
        guess = 3.99717671842943;
    } else if max >= 2.0 {
        guess = 7.99435343685886;
    } else if max >= 1.5 {
        guess = 13.6664;
    } else if max >= 1.25 {
        guess = 24.8327;
    } else if max >= 1.125 {
        guess = 47.0464;
    } else {
        guess = 91.4029;
    }
    let mut lns = [0.0; 3];    
    for i in 0..3 {
        lns[i] = rands[i].ln();
    }
        
    
    /* 
        Use Newton's method to solve for n rand[0]^n+rand[1]y^n+rand[2]^n = 255.0
    */
    #[cfg(test)]
    let mut count = 0;
    let mut pow = [0.0;3];
    loop {
        let mut sum = 0.0;
        let mut div = 0.0;
        for i in 0..3 { // find both the d/dx and value
            pow[i] = rands[i].powf(guess);
            sum += pow[i];
            div += pow[i] * lns[i];

        }
        let dif = (sum - 256.0) / div;
        #[cfg(test)]
        {   
            println!("iter {}: guess: {} dif: {} pow: {:?}", count, guess, sum - 256.0, pow); 
            count+=1;
        }
        if dif.abs() <= std::f32::EPSILON {
            break;
        }
        guess = guess - dif;
    }
    [pow[0] - 1.0, pow[1] - 1.0, pow[2] - 1.0, 0.0]
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_calc_nths_50_100_150() {
        let res = calc_nths([50.0,100.0,150.0]);
        assert_eq!(res,[42.80379, 84.57778, 125.61844, 0.0]);
    }

    #[test]
    fn test_calc_nths_100_100_100() {
        let res = calc_nths([100.0, 100.0, 100.0]);
        assert_eq!(res, [84.333336, 84.333336, 84.333336, 0.0]);
    }

    #[test]
    fn test_less_2() {
        let res = calc_nths([1.6545649, 0.29497832, 0.6539477]);
        assert_eq!(res, [254.99069, -0.99999857, -0.9906955, 0.0]);
    }
}

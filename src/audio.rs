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

#[derive(Debug)]
pub struct PendingProducer {
    sample_size: usize,
    sender: mpsc::SyncSender<StereoSample>,
    recv: mpsc::Receiver<StereoSample>,
    source: InactiveSource
}

#[derive(Debug)]
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
    pub fn is_no_sig(&self) -> bool {
        (self.left[0] == 0.0) && (self.right[0] == 0.0)
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
            /* TODO: evaluate potential performance gains by eliminating allocation, 
                that will be dropped every 8 audio frames. This is probably 
                expensive and could in theory be replace by either some kind of 
                mutex protected circular buffer or perhaps passing allocated Vec 
                in a return mpsc-channel. Profiling needs to be done to determine 
                where this thread is actually spending time. It could be that this 
                is just a neccassaryly expensive component.
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

pub struct Notifications;

impl jack::NotificationHandler for Notifications {
    fn xrun(&mut self, _: &jack::Client) -> jack::Control {
        //eprintln!("xrun occurred");
        jack::Control::Continue
    }
}

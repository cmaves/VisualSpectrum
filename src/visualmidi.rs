use rs_ws281x::Controller;
use crate::midi::{ActiveSource,InactiveSource,MidiActive,MidiEvent,MidiMessage};
use crate::audio::SpectrumError;
use crate::led::{PulseCalc};
use jack::Client;
use std::collections::VecDeque;
use std::sync::mpsc::sync_channel;
use std::thread::spawn;
use std::time::Instant;
use random_color::RandomColor;

pub struct MidiCon {
    controller: Controller,
    invert: bool,
    brightness: f32,
    pulse: bool,
}

impl MidiCon
{
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
        MidiCon {
            controller: controller,
            invert: invert,
            pulse: pulse,
            brightness: brightness
        }
    }
}
pub trait MidiRenderer<T> {
    type TerminateType;
    fn display(mut self, client: T) -> Result<Self::TerminateType, SpectrumError>;
}
struct Color ([u8; 4]);
impl Color {
    fn new(color: [u32; 3]) -> Self {
        let mut ret = [0; 4];
        ret[0] = (color[0] >> 24) as u8;
        ret[1] = (color[1] >> 24) as u8;
        ret[2] = (color[2] >> 24) as u8;
        Color(ret)
    }
    fn scale(&self, scalar: f32) -> Color {
        let mut ret = [0; 4];
        for i in 0..3 {
            ret[0] = (self.0[i] as f32 * scalar) as u8;
        }
        Color(ret)
    }
    fn add(&self, color: &Self) -> Self {
        let mut ret = [0; 4];
        for i in 0..3 {
            ret[i] = if let Some(v) = self.0[i].checked_add(color.0[i]) {
                v
            } else {
                255
            };
        }
        Color(ret)
    }
}
struct Note {
    on: bool,
    note: u8,
    vel: u8,
    on_time: u64,
    off_time: u64,
    so_time: u64,
    color: Color
}
impl Note {
    fn calc_leds(&self, cur_time: u64, leds: &mut Vec<[u8; 4]>) -> u64 {
        let ret = self.calc_tr(cur_time);
        let start = (if self.note >= 7 { self.note - 7 } else { 0 }) as usize;
        let vr = (self.vel as f32) / 127.0;
        let mut end = start + 15;
        if end > leds.len() { end = leds.len() }
        for i in start..end {
            let diff  = abs_diff_u(start, self.note as usize);
            let tr = ret as f32 / 5000000.0; // time remaining ratio
            let ratio = (2_f32.powf(diff as f32 * tr * vr / 7.0) - 1.0) / 255.0;
            let color = self.color.scale(ratio);
            leds[i] = Color(leds[i]).add(&color).0;
        }
        ret
    }
    fn calc_tr(&self, cur_time: u64) ->  u64 {
        if self.on {
            let mut diff = cur_time - self.on_time;
            if diff > 5000000 {
                diff = 5000000;
            }
            diff
        } else {
            let diff = self.off_time - self.on_time;
            if let Some(v) = diff.checked_sub((cur_time - self.off_time) * 5) {
                v 
            } else {
                0
            }
        }
    }
}
impl<T, U, V> MidiRenderer<T> for MidiCon
    where T:    InactiveSource<ActiveType=U>,
                U: 'static + Send + Iterator<Item=MidiMessage> + ActiveSource<InactiveType=V>,
                V: 'static + Send
{
    type TerminateType = <<T as InactiveSource>::ActiveType as ActiveSource>::InactiveType;
    fn display(mut self, client: T) -> Result<Self::TerminateType, SpectrumError>
    {
        let mut active = client.activate()?;
        let (sender, recv) = sync_channel(1);
        let brightness = self.brightness;
        let invert = self.invert;
        let pulse = self.pulse;
        let led_num = self.controller.leds(0).len();
        let calc_thread = spawn(move || {
            let mut pc = PulseCalc::new(brightness);
            let now = Instant::now();
            let mut notes = VecDeque::new();
            let mut sustain = false;
            let rc = RandomColor::new();
            loop {
                //if start_time % 512 == 0 { pc.new_color(); }
                loop {
                    let mm = match active.next() {
                        Some(mm) => mm,
                        None => break
                    };
                    match mm.event {
                        MidiEvent::NoteOn(n,v) => notes.push_back( Note {
                            on: true, note: n, vel: v, on_time: mm.time, off_time: 0,
                            color: Color::new(rc.to_rgb_array()), so_time: 0
                        }),
                        MidiEvent::NoteOff(n,_) => {
                            for note in notes.iter_mut() {
                                if note.on && note.note == n {
                                    note.on = false;
                                    note.off_time = mm.time;                    
                                    break;
                                }
                            }
                        },
                        MidiEvent::Unrecongized => return active.deactivate(),
                        _ => {},
                    }
                    println!("{:?}", mm);
                }

                let mut leds = vec![[0, 0, 0, 0]; 300];//Vec::with_capacity(300);
                let cur_time = active.cur_time();
                for note in notes.iter() {
                    note.calc_leds(cur_time, &mut leds);
                }
                if let Err(_) = sender.send(leds) {
                    return active.deactivate();
                }
                let mut count = 0;
                let res = if pulse {
                    
                } else {
                };
                
            }
        });

        // render the new field in 
        loop {
            let vals = match recv.recv() {
                Ok(o) => o,
                Err(_) => break
            };
            let leds = self.controller.leds_mut(0);
            for i in 0..leds.len() {
                for j in 0..3 {
                    leds[i][j] = vals[i][j];
                }
            }
            if let Err(_) = self.controller.render() {
                break;
            }
        }

        // return the deactivated JACK client
        if let Ok(v) = calc_thread.join() {
            v
        } else {
            Err(SpectrumError::SpectrumError("calc_thread panicked!"))
        }
    }
}

fn abs_diff_u(x: usize, y: usize) -> usize {
    if x > y { x - y } else { y - x }
}

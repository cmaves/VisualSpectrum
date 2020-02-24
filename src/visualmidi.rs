use rs_ws281x::Controller;
use crate::midi::{ActiveSource,InactiveSource,MidiEvent,MidiMessage};
use crate::audio::SpectrumError;
//TODO: implement pulse use crate::led::{PulseCalc};
use std::collections::VecDeque;
use std::sync::mpsc::sync_channel;
use std::thread::{spawn,sleep};
use std::time::{Duration,Instant};
use random_color::RandomColor;

pub struct MidiCon {
    controller: Controller,
    verbose: bool,
    pulse: bool,
    lc: LedConfigData,
}

impl MidiCon
{
    pub fn new(pin: i32, count: i32, pulse: bool) -> Self {

        let channel = rs_ws281x::ChannelBuilder::new()
            .pin(pin).strip_type(rs_ws281x::StripType::Ws2812)
            .count(count)
            .brightness(255).build();
        let controller = rs_ws281x::ControllerBuilder::new()
            .freq(800_000)
            .channel(0, channel).build().unwrap();
        MidiCon {
            controller,
            verbose: false,
            pulse,
            lc: LedConfigData::default()
        }
    }
    pub fn set_verbose(&mut self, verbose: bool) { self.verbose = verbose; }
}
pub trait LedConfig {
    fn set_config<T: LedConfig>(&mut self, config: &T);
    fn set_invert(&mut self, invert: bool);
    fn set_fade_out(&mut self, fade_out: u32);
    fn set_fade_in(&mut self, fade_in: u32);
    fn set_brightness(&mut self, brightnesss: u8);
    fn get_fade_out(&self) -> u32;
    fn get_fade_in(&self) -> u32;
    fn get_brightness(&self) -> u8;
    fn get_invert(&self) -> bool;
    fn get_config(&self) -> LedConfigData;
}

#[derive(Clone,Copy)]
pub struct LedConfigData {
    invert: bool,
    brightness: u8,
    fade_out: u32,
    fade_in: u32
}
impl Default for LedConfigData {
    fn default() -> Self {  
        LedConfigData { brightness: 255, fade_out: 3000000, fade_in: 200000, invert: false }
    }
}
impl LedConfig for LedConfigData {
    fn set_config<T: LedConfig>(&mut self, config: &T) { 
        *self = config.get_config();
        /*self.fade_out = config.get_fade_out(); 
        self.fade_in = config.get_fade_in();
        self.brightness = config.get_brightness();*/
    }
    fn set_fade_out(&mut self, fade_out: u32) { self.fade_out = fade_out; }
    fn set_fade_in(&mut self, fade_in: u32) { self.fade_in = fade_in; }
    fn set_brightness(&mut self, brightness: u8) { self.brightness = brightness; }
    fn set_invert(&mut self, invert: bool) { self.invert = invert }
    fn get_fade_out(&self) -> u32 { self.fade_out }
    fn get_fade_in(&self) -> u32 { self.fade_in }
    fn get_brightness(&self) -> u8 { self.brightness }
    fn get_config(&self) -> LedConfigData { *self }
    fn get_invert(&self) -> bool { self.invert }

}
impl LedConfig for MidiCon {
    fn set_config<T: LedConfig>(&mut self, config: &T) { self.lc = config.get_config(); }
    fn set_fade_out(&mut self, fade_out: u32) { self.lc.fade_out = fade_out; }
    fn set_fade_in(&mut self, fade_in: u32) { self.lc.fade_in = fade_in; }
    fn set_brightness(&mut self, brightness: u8) { self.lc.brightness = brightness; }
    fn set_invert(&mut self, invert: bool) { self.lc.invert = invert }
    fn get_fade_out(&self) -> u32 { self.lc.fade_out }
    fn get_fade_in(&self) -> u32 { self.lc.fade_in }
    fn get_brightness(&self) -> u8 { self.lc.brightness }
    fn get_invert(&self) -> bool { self.lc.invert }
    fn get_config(&self) -> LedConfigData { self.lc }
}
pub trait MidiRenderer<T> {
    type TerminateType;
    fn display(self, client: T) -> Result<Self::TerminateType, SpectrumError>;
}
#[derive(Debug)]
struct Color ([u8; 4]);
impl Color {
    fn new(color: [u32; 3]) -> Self {
        println!("color: {:?}", color);
        let mut ret = [0; 4];
        ret[0] = color[0] as u8;
        ret[1] = color[1] as u8;
        ret[2] = color[2] as u8;
        Color(ret)
    }
    /*
    fn scale(&self, scalar: f32) -> Color {
        let mut ret = [0; 4];
        for i in 0..3 {
            ret[i] = (self.0[i] as f32 * scalar) as u8;
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
    */
}
#[derive(Debug)]
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
    fn calc_leds<M: LedConfig>(&self, config: &M, cur_time: u64, leds: &mut [[u8; 4]]) -> bool {
        let time_past = if cur_time < self.on_time { // in this case we are early
            return true;
        } else if self.on { // in this case the note is still on an fading
            cur_time - self.on_time 
        } else {
            match cur_time.checked_sub(self.off_time) {
                Some(v) => { // the note is off
                    (self.off_time - self.on_time) + v * 10
                },
                None => { // the note has been set to off but the time has not elapsed yet
                    cur_time - self.on_time
                }
            }
        };
        let (fade_out, fade_in) = (config.get_fade_out() as u64, config.get_fade_in() as u64);
        let total = fade_out + fade_in;
        // power represents the how strong the note is based on elapsed time and velocity
        // div represents the final dividend.
        let (power, div) = if time_past > total { // we have finished this note
            return false;
        } else if time_past > fade_in { // we are fading down
            (total - time_past, fade_out * 128 * 256) // this 8 is to account for the spread from the center led
        } else { // we are fading in
            (time_past, fade_in * 128 * 256)
        };            
        let mut base = [0; 4];
        for i in 0..3 {
            base[i] = self.color.0[i] as u64 * power * self.vel as u64;
        }
        let center = self.note as usize * 2;
        for i in 0..8 {
            // calculate the final color based on the distance from the center led
            let mut color = [0; 4];
            for j in 0..3 {
                color[j] = ((base[j] << (8 - i)) / div) as u8;
            } 
            
            // actually set the led on either side of the center led
            if let Some(first) = center.checked_sub(i) {
                for j in 0..3 {
                    leds[first][j] = leds[first][j].saturating_add(color[j]);
                }
            }
            if i == 0 { continue; } // this is duplicative on the zero iteration.
            let last = center + i;
            if last < leds.len() {
                for j in 0..3 {
                    leds[last][j] = leds[last][j].saturating_add(color[j]);
                }
            }
        }
        true 
    }
    fn set_off_time<T: LedConfig>(&mut self, off_time: u64, config: &T) -> bool {
        if ! self.on { return false; }
        let min = self.on_time + config.get_fade_in() as u64;
        self.off_time = if off_time > min {
            off_time 
        } else {
            min
        };
        self.on = false;
        true
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
        let mut active = client.activate()?; // start receiving midi notes
        let (sender, recv) = sync_channel(1); // setup the sync channel for the leds 
        let config = self.get_config(); 
        let calc_thread = spawn(move || {
            //TODO: implement pulse  let mut pc = PulseCalc::new(config.get_brightness() as f32 / 255.0);
            // let now = Instant::now();
            let mut notes = VecDeque::new();
            //TODO: implement sustain let mut sustain = false;
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
                                if note.note == n { 
                                     if note.set_off_time(mm.time, &config) { break; }
                                }
                            }
                        },
                        MidiEvent::Unrecongized => return active.deactivate(),
                        _ => {},
                    }
                    println!("{:?}", mm);
                }
                let mut leds = [[0, 0, 0, 0]; 256];
                let cur_time = active.cur_time();
                let mut start = std::usize::MAX;
                for note in notes.iter().enumerate() {
                    let tr = note.1.calc_leds(&config, cur_time, &mut leds[..]);
                    if start == std::usize::MAX && tr {
                        start = note.0;
                    }
                }
                for _ in 0..start {
                    if let Some(v)= notes.pop_front() {
                        println!("{:?}", v);
                    } else {
                        break;
                    }
                }
                if let Err(_) = sender.send(leds) {
                    return active.deactivate();
                }
                //TODO: pulse
                
            }
        });

        // render the new field in 
        let mut count = 0;
        let mut start_fps = Instant::now();
        let min_frame_time = Duration::from_secs_f32(1.0 / 64.0); // updating too quickly causes led flicker
        loop {
    
            let vals = match recv.recv() {
                Ok(o) => o,
                Err(_) => break
            };
            let end_time = Instant::now() + min_frame_time;
            let leds = self.controller.leds_mut(0);
            let end = if vals.len() > leds.len() {
                leds.len()
            } else {
                vals.len()
            };
            for i in 0..end {
                for j in 0..3 {
                    leds[i][j] = vals[i][j];
                }
            }
            if let Err(_) = self.controller.render() {
                break;
            }
            if self.verbose {
                count += 1;
                if count % 1200 == 0 {
                    let elapsed = start_fps.elapsed().as_secs_f32();
                    let fps = 1200.0 / elapsed;
                    println!("{:.2} fps, {:.4} ms/update", fps, elapsed);
                    start_fps = Instant::now();
                }
            }
            sleep(end_time.saturating_duration_since(Instant::now()));
        }

        // return the deactivated JACK client
        if let Ok(v) = calc_thread.join() {
            v
        } else {
            Err(SpectrumError::SpectrumError("calc_thread panicked!"))
        }
    }
}

pub fn abs_diff_u(x: usize, y: usize) -> usize {
    if x > y { x - y } else { y - x }
}


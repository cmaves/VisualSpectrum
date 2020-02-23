use crate::audio::{Notifications,SpectrumError};
use jack::{AsyncClient,Client,Control,MidiIn,Port,ProcessHandler,RawMidi};
use std::sync::mpsc::{sync_channel,Receiver,SyncSender,TryRecvError};

pub struct MidiActive {
    recv: Receiver<MidiMessage>,
    source: AsyncClient<Notifications,MidiHandler>
}
impl ActiveSource for MidiActive {
    type InactiveType = Client;
    fn deactivate(self) -> Result<Self::InactiveType, SpectrumError> {
        match self.source.deactivate() {
            Ok(ok) => Ok(ok.0),
            Err(err) => Err(SpectrumError::JackError(err))
        }
    }
    fn cur_time(&self) -> u64 {
        let client = self.source.as_client();
        client.frames_to_time(client.frame_time())
    }
}

impl Iterator for MidiActive {
    type Item = MidiMessage;
    fn next(&mut self) -> Option<Self::Item> {
        match self.recv.try_recv() {
            Ok(ok) => Some(ok),
            Err(err) => match err {
                TryRecvError::Disconnected => Some(MidiMessage {
                    event: MidiEvent::Unrecongized,
                    time: 0,
                }),
                TryRecvError::Empty => None
            }
        }
    }
}

pub trait ActiveSource {
    type InactiveType;
    fn deactivate(self) -> Result<Self::InactiveType, SpectrumError>;
    fn cur_time(&self) -> u64;
}

#[derive(Debug)]
pub enum MidiEvent {
    NoteOn(u8,u8),
    NoteOff(u8,u8),
    PlyPrs(u8,u8),
    CtrlChg(u8,u8),
    ProgChg(u8),
    ChnlPrs(u8),
    PitchWheel(u16),
    RealTime(u8),
    Unrecongized
}
fn raw_to_mm(rm: RawMidi, time: u64) -> MidiMessage {
    let event = match rm.bytes[0] & 0xF0 {
        0x80 => MidiEvent::NoteOff(rm.bytes[1], rm.bytes[2]),
        0x90 => MidiEvent::NoteOn(rm.bytes[1], rm.bytes[2]),
        0xA0 => MidiEvent::PlyPrs(rm.bytes[1], rm.bytes[2]),
        0xB0 => MidiEvent::CtrlChg(rm.bytes[1], rm.bytes[2]),
        0xC0 => MidiEvent::ProgChg(rm.bytes[1]),
        0xD0 => MidiEvent::ChnlPrs(rm.bytes[1]),
        0xE0 => MidiEvent::PitchWheel((rm.bytes[1] as u16) + ((rm.bytes[2] as u16) << 8)),
        0xF0 => MidiEvent::RealTime(rm.bytes[0] & 0xF),
        _ => MidiEvent::Unrecongized
    };
    MidiMessage {
        time: time,
        event: event
    } 
}
#[derive(Debug)]
pub struct MidiMessage {
    pub time: u64,
    pub event: MidiEvent
}

pub trait InactiveSource {
    type ActiveType: Iterator<Item=MidiMessage> + ActiveSource;
    fn activate(self) -> Result<Self::ActiveType, SpectrumError>;
}


impl InactiveSource for Client {
    type ActiveType = MidiActive;
    fn activate(self) -> Result<Self::ActiveType, SpectrumError> {
        let input = match self.register_port("midi_in", MidiIn) {
            Ok(ok) => ok,
            Err(err) => return Err(SpectrumError::JackError(err))
        };
        let (sender, recv) = sync_channel(64);
        let handler = MidiHandler {
            sender: sender,
            input: input,
            count: 0,
            fps: self.time_to_frames(1000000),
            frame_len: self.frames_to_time(self.buffer_size())
        };
        match self.activate_async(Notifications, handler) {
            Ok(ok) => {
                Ok(MidiActive {
                    recv: recv,
                    source: ok
                })
            },
            Err(err) => Err(SpectrumError::JackError(err))
        }
    }

}

impl ProcessHandler for MidiHandler {
    fn process(&mut self, client: &jack::Client, ps: &jack::ProcessScope) -> Control {
    let time = ps.last_frame_time();
        for i in self.input.iter(ps) {
            let mm = raw_to_mm(i, client.frames_to_time(i.time + time));
            if let MidiEvent::RealTime(u) = mm.event {
                if u == 0x8  || u == 0xE {
                    continue;
                }
            }
            self.sender.send(mm).unwrap();
        }
        self.count += 1;
        Control::Continue
    }
}

pub struct MidiHandler {
    sender: SyncSender<MidiMessage>,
    input: Port<MidiIn>,
    frame_len: u64,
    count: u64,
    fps: u32
}
pub fn new_jack() -> Result<Client, SpectrumError> {
        match jack::Client::new("spectrum", jack::ClientOptions::NO_START_SERVER) {
            Ok(ok) => Ok(ok.0),
            Err(err) => Err(SpectrumError::JackError(err))
        }
    }


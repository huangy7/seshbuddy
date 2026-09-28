const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const OSC_INTRO: u8 = b']';
const ST_FINAL: u8 = b'\\';
const OSC_MAX: usize = 2048;
const SESHBUDDY_MARKER: &[u8] = b"notify;SeshBuddy;";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Esc,
    Osc,
    OscEsc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Working,
    Waiting,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Transition {
    Started,
    Working,
    Attention,
    Finished,
    Exited,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AgentDetector {
    state: State,
    osc: Vec<u8>,
    armed: bool,
    status: Status,
}

impl AgentDetector {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            osc: Vec::new(),
            armed: false,
            status: Status::Working,
        }
    }

    pub fn process<F: FnMut(Transition)>(&mut self, input: &[u8], mut emit: F) {
        if self.state == State::Ground && !input.contains(&ESC) {
            return;
        }
        for &b in input {
            match self.state {
                State::Ground => {
                    if b == ESC {
                        self.state = State::Esc;
                    }
                }
                State::Esc => match b {
                    OSC_INTRO => {
                        self.state = State::Osc;
                        self.osc.clear();
                    }
                    ESC => {}
                    _ => self.state = State::Ground,
                },
                State::Osc => match b {
                    BEL => {
                        self.finish_osc(&mut emit);
                        self.state = State::Ground;
                    }
                    ESC => self.state = State::OscEsc,
                    _ => {
                        if self.osc.len() < OSC_MAX {
                            self.osc.push(b);
                        } else {
                            self.osc.clear();
                            self.state = State::Ground;
                        }
                    }
                },
                State::OscEsc => match b {
                    ST_FINAL => {
                        self.finish_osc(&mut emit);
                        self.state = State::Ground;
                    }
                    ESC | _ => {
                        self.osc.clear();
                        self.state = State::Ground;
                    }
                },
            }
        }
    }

    pub fn finish<F: FnMut(Transition)>(&mut self, mut emit: F) {
        if self.armed {
            self.disarm();
            emit(Transition::Exited);
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
        self.status = Status::Working;
    }

    fn finish_osc<F: FnMut(Transition)>(&mut self, emit: &mut F) {
        let body = std::mem::take(&mut self.osc);
        let (ps, pt) = match body.iter().position(|&c| c == b';') {
            Some(i) => (&body[..i], &body[i + 1..]),
            None => (&body[..], &body[0..0]),
        };
        match ps {
            b"133" => self.handle_osc133(pt, emit),
            b"777" => self.handle_osc777(pt, emit),
            _ => {}
        }
    }

    fn handle_osc777<F: FnMut(Transition)>(&mut self, pt: &[u8], emit: &mut F) {
        let event = pt.strip_prefix(SESHBUDDY_MARKER);
        if let Some(event) = event {
            match event {
                b"working" => {
                    let was_unarmed = !self.armed;
                    self.ensure_armed(emit);
                    if was_unarmed || self.status != Status::Working {
                        self.status = Status::Working;
                        emit(Transition::Working);
                    }
                }
                b"attention" => {
                    self.ensure_armed(emit);
                    if self.status != Status::Waiting {
                        self.status = Status::Waiting;
                        emit(Transition::Attention);
                    }
                }
                b"finished" => {
                    self.ensure_armed(emit);
                    if self.status != Status::Waiting {
                        self.status = Status::Waiting;
                        emit(Transition::Finished);
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_osc133<F: FnMut(Transition)>(&mut self, pt: &[u8], emit: &mut F) {
        match pt.first() {
            Some(b'C') => {
                if self.armed {
                    return;
                }
                self.armed = true;
                self.status = Status::Working;
                emit(Transition::Started);
            }
            Some(b'D') if self.armed => {
                self.disarm();
                emit(Transition::Exited);
            }
            _ => {}
        }
    }

    fn ensure_armed<F: FnMut(Transition)>(&mut self, emit: &mut F) {
        if !self.armed {
            self.armed = true;
            emit(Transition::Started);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: &mut AgentDetector, input: &[u8]) -> Vec<Transition> {
        let mut out = Vec::new();
        d.process(input, |t| out.push(t));
        out
    }

    fn osc(body: &str) -> Vec<u8> {
        let mut v = vec![ESC, OSC_INTRO];
        v.extend_from_slice(body.as_bytes());
        v.extend_from_slice(&[ESC, ST_FINAL]);
        v
    }

    #[test]
    fn seshbuddy_marker_drives_status() {
        let mut d = AgentDetector::new();
        assert_eq!(
            run(&mut d, &osc("777;notify;SeshBuddy;attention")),
            vec![Transition::Started, Transition::Attention]
        );
        assert_eq!(
            run(&mut d, &osc("777;notify;SeshBuddy;working")),
            vec![Transition::Working]
        );
        assert!(run(&mut d, &osc("777;notify;SeshBuddy;working")).is_empty());
        assert_eq!(
            run(&mut d, &osc("777;notify;SeshBuddy;finished")),
            vec![Transition::Finished]
        );
    }

    #[test]
    fn ignores_non_seshbuddy_marker() {
        let mut d = AgentDetector::new();
        assert!(run(&mut d, &osc("777;notify;Other;attention")).is_empty());
    }

    #[test]
    fn osc133_triggers_started_and_exited() {
        let mut d = AgentDetector::new();
        assert_eq!(run(&mut d, &osc("133;C")), vec![Transition::Started]);
        assert_eq!(run(&mut d, &osc("133;D;0")), vec![Transition::Exited]);
    }

    #[test]
    fn finish_reports_exited_when_armed() {
        let mut d = AgentDetector::new();
        run(&mut d, &osc("133;C"));
        let mut out = Vec::new();
        d.finish(|t| out.push(t));
        assert_eq!(out, vec![Transition::Exited]);
    }

    #[test]
    fn first_working_event_emits_started_and_working() {
        let mut d = AgentDetector::new();
        assert_eq!(
            run(&mut d, &osc("777;notify;SeshBuddy;working")),
            vec![Transition::Started, Transition::Working]
        );
    }

    #[test]
    fn double_esc_in_osc_aborts() {
        let mut d = AgentDetector::new();
        run(&mut d, &osc("133;C"));
        let mut seq = vec![ESC, OSC_INTRO];
        seq.extend_from_slice(b"777;notify;SeshBuddy;working");
        seq.push(ESC);
        seq.push(ESC);
        seq.push(ST_FINAL);
        assert!(run(&mut d, &seq).is_empty());
    }
}

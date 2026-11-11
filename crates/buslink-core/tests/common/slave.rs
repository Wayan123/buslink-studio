//! A minimal multi-unit RTU slave for core tests (the full simulator lives in
//! crates/buslink-sim, which core cannot depend on). Test code only.
#![allow(dead_code, clippy::unwrap_used)]

use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{ExceptionCode, ReadData, ReadRequest, Table, encode_read_response, exception_response};
use buslink_core::protocol::rtu;
use buslink_core::transport::fake::FakeReply;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct Unit {
    pub alive: bool,
    pub holding: BTreeMap<u16, u16>,
    pub input: BTreeMap<u16, u16>,
    pub coils: BTreeMap<u16, bool>,
    /// Ignore writes (acknowledge but do not change state) to test read-back mismatch.
    pub sticky: bool,
    /// Answer writes with this exception.
    pub write_exception: Option<u8>,
    /// Do not answer writes (timeout) but apply them.
    pub silent_writes: bool,
    pub writes_seen: usize,
}

#[derive(Clone, Default)]
pub struct Bus {
    pub units: Arc<Mutex<HashMap<u8, Unit>>>,
    pub delay_ms: u64,
    pub chunk: usize,
}

impl Bus {
    pub fn new() -> Self {
        Self { units: Arc::default(), delay_ms: 10, chunk: 0 }
    }

    pub fn add(&self, unit: u8, holding: &[(u16, u16)]) {
        let u = Unit { alive: true, holding: holding.iter().copied().collect(), ..Unit::default() };
        self.units.lock().unwrap().insert(unit, u);
    }

    pub fn set_alive(&self, unit: u8, alive: bool) {
        self.units.lock().unwrap().get_mut(&unit).unwrap().alive = alive;
    }

    pub fn with<R>(&self, unit: u8, f: impl FnOnce(&mut Unit) -> R) -> R {
        f(self.units.lock().unwrap().get_mut(&unit).unwrap())
    }

    pub fn responder(&self) -> Box<dyn FnMut(&[u8]) -> FakeReply + Send> {
        let bus = self.clone();
        Box::new(move |adu| bus.answer(adu))
    }

    fn reply(&self, unit: u8, pdu: Vec<u8>) -> FakeReply {
        FakeReply::Bytes { data: rtu::encode_adu(unit, &pdu).unwrap(), delay_ms: self.delay_ms, chunk: self.chunk }
    }

    pub fn answer(&self, adu: &[u8]) -> FakeReply {
        let Ok((unit, pdu)) = rtu::decode_adu(adu) else { return FakeReply::Silence };
        let mut units = self.units.lock().unwrap();
        let Some(u) = units.get_mut(&unit) else { return FakeReply::Silence };
        if !u.alive {
            return FakeReply::Silence;
        }
        let fc = pdu[0];
        if let Ok(req) = ReadRequest::decode(pdu) {
            let addrs = req.address()..=req.end_address();
            let data = match req.table() {
                Table::HoldingRegister | Table::InputRegister => {
                    let map = if req.table() == Table::HoldingRegister { &u.holding } else { &u.input };
                    let regs: Option<Vec<u16>> = addrs.map(|a| map.get(&a).copied()).collect();
                    regs.map(ReadData::Registers)
                }
                Table::Coil | Table::DiscreteInput => {
                    let bits: Option<Vec<bool>> = addrs.map(|a| u.coils.get(&a).copied()).collect();
                    bits.map(ReadData::Bits)
                }
            };
            let pdu = match data {
                Some(d) => encode_read_response(&req, &d).unwrap(),
                None => exception_response(fc, ExceptionCode::ILLEGAL_DATA_ADDRESS).to_vec(),
            };
            return self.reply(unit, pdu);
        }
        match WriteRequest::decode(pdu) {
            Ok(w) => {
                u.writes_seen += 1;
                if let Some(code) = u.write_exception {
                    return self.reply(unit, exception_response(fc, ExceptionCode(code)).to_vec());
                }
                if !u.sticky {
                    match &w {
                        WriteRequest::SingleRegister { address, value } => {
                            u.holding.insert(*address, *value);
                        }
                        WriteRequest::MultipleRegisters { address, values } => {
                            for (i, v) in values.iter().enumerate() {
                                u.holding.insert(address + i as u16, *v);
                            }
                        }
                        WriteRequest::SingleCoil { address, on } => {
                            u.coils.insert(*address, *on);
                        }
                        WriteRequest::MultipleCoils { address, values } => {
                            for (i, v) in values.iter().enumerate() {
                                u.coils.insert(address + i as u16, *v);
                            }
                        }
                    }
                }
                if u.silent_writes {
                    return FakeReply::Silence;
                }
                self.reply(unit, w.response_pdu())
            }
            Err(e) => self.reply(unit, exception_response(fc, e.exception()).to_vec()),
        }
    }
}

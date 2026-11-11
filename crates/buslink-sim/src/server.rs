//! Server-side request dispatch for one simulated device.
//!
//! Reads use the core `ReadRequest` decoder; writes use the command service's
//! wire decoder (this crate may handle write PDUs, safety rule 2). A write
//! that touches any unmapped or read-only address is rejected as a whole.

use crate::devices::SimModel;
use buslink_core::command::wire::WriteRequest;
use buslink_core::protocol::pdu::{
    ExceptionCode, ReadData, ReadFunction, ReadRequest, Table, encode_read_response, exception_response,
};

fn exception(function: u8, code: ExceptionCode) -> Vec<u8> {
    exception_response(function, code).to_vec()
}

/// Answer one request PDU. Always returns a reply PDU (unicast).
pub fn handle(model: &mut dyn SimModel, pdu: &[u8], wrong_order: bool) -> Vec<u8> {
    let Some(&fc) = pdu.first() else {
        return exception(0, ExceptionCode::ILLEGAL_FUNCTION);
    };
    if !model.supports(fc) {
        return exception(fc, ExceptionCode::ILLEGAL_FUNCTION);
    }
    if ReadFunction::from_code(fc).is_some() {
        return read(model, pdu, fc, wrong_order);
    }
    write(model, pdu, fc)
}

fn read(model: &mut dyn SimModel, pdu: &[u8], fc: u8, wrong_order: bool) -> Vec<u8> {
    let req = match ReadRequest::decode(pdu) {
        Ok(r) => r,
        Err(e) => return exception(fc, e.exception()),
    };
    let image = model.image(wrong_order);
    let table = req.table();
    let words: Option<Vec<u16>> = (req.address()..=req.end_address())
        .map(|a| image.word(table, a))
        .collect();
    let Some(words) = words else {
        return exception(fc, ExceptionCode::ILLEGAL_DATA_ADDRESS);
    };
    let data = if table.is_bit() {
        ReadData::Bits(words.iter().map(|w| *w != 0).collect())
    } else {
        ReadData::Registers(words)
    };
    encode_read_response(&req, &data).unwrap_or_else(|_| exception(fc, ExceptionCode::SERVER_DEVICE_FAILURE))
}

fn write(model: &mut dyn SimModel, pdu: &[u8], fc: u8) -> Vec<u8> {
    let w = match WriteRequest::decode(pdu) {
        Ok(w) => w,
        Err(e) => return exception(fc, e.exception()),
    };
    let image = model.image(false);
    let (table, addresses): (Table, Vec<u16>) = match &w {
        WriteRequest::SingleCoil { address, .. } => (Table::Coil, vec![*address]),
        WriteRequest::MultipleCoils { address, values } => (
            Table::Coil,
            (0..values.len()).map(|i| address.wrapping_add(i as u16)).collect(),
        ),
        WriteRequest::SingleRegister { address, .. } => (Table::HoldingRegister, vec![*address]),
        WriteRequest::MultipleRegisters { address, values } => (
            Table::HoldingRegister,
            (0..values.len()).map(|i| address.wrapping_add(i as u16)).collect(),
        ),
    };
    if !addresses.iter().all(|a| image.is_writable(table, *a)) {
        return exception(fc, ExceptionCode::ILLEGAL_DATA_ADDRESS);
    }
    match model.apply(&w) {
        Ok(()) => w.response_pdu(),
        Err(code) => exception(fc, code),
    }
}

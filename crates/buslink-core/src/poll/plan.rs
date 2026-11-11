//! Block merging: adjacent tags of one table become one read, within the FC
//! limits (125 registers / 2000 bits) or a smaller device limit. A tag is
//! never split, and with `max_gap = 0` no address outside a tag is read.

use crate::device::DeviceInstance;
use crate::profile::Profile;
use crate::protocol::pdu::{MAX_READ_BITS, MAX_READ_REGISTERS, ReadRequest, Table};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanLimits {
    pub max_registers: u16,
    pub max_bits: u16,
    /// Unused addresses a block may span between tags.
    pub max_gap: u16,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self { max_registers: MAX_READ_REGISTERS, max_bits: MAX_READ_BITS, max_gap: 0 }
    }
}

impl PlanLimits {
    pub fn for_device(d: &DeviceInstance) -> Self {
        Self {
            max_registers: d.max_registers_per_read.unwrap_or(MAX_READ_REGISTERS).clamp(1, MAX_READ_REGISTERS),
            max_bits: d.max_bits_per_read.unwrap_or(MAX_READ_BITS).clamp(1, MAX_READ_BITS),
            max_gap: d.max_gap,
        }
    }

    fn limit(&self, table: Table) -> u16 {
        if table.is_bit() { self.max_bits } else { self.max_registers }
    }
}

/// Where one tag sits inside a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockTag {
    /// Index into `profile.registers`.
    pub register: usize,
    /// First word/bit of the tag inside the block's data.
    pub offset: u16,
    pub width: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadBlock {
    pub request: ReadRequest,
    pub tags: Vec<BlockTag>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub blocks: Vec<ReadBlock>,
    /// Tags that cannot be read within the limits (wider than the device read limit).
    pub unreadable: Vec<usize>,
}

/// Plan the reads for one device.
pub fn plan(profile: &Profile, limits: PlanLimits) -> Plan {
    let mut out = Plan::default();
    for table in Table::ALL {
        let mut spans: Vec<(u32, u32, usize)> = profile
            .registers
            .iter()
            .enumerate()
            .filter(|(_, r)| r.table == table)
            .map(|(i, r)| (u32::from(r.address), r.end_address(), i))
            .collect();
        spans.sort();
        let limit = u32::from(limits.limit(table));
        let mut current: Option<(u32, u32, Vec<usize>)> = None;
        for (start, end, idx) in spans {
            if end - start + 1 > limit || end > 65_535 {
                out.unreadable.push(idx);
                continue;
            }
            current = match current.take() {
                Some((s, e, mut tags)) if start <= e + 1 + u32::from(limits.max_gap) && end.max(e) - s < limit => {
                    tags.push(idx);
                    Some((s, end.max(e), tags))
                }
                Some(done) => {
                    push_block(&mut out, profile, table, done);
                    Some((start, end, vec![idx]))
                }
                None => Some((start, end, vec![idx])),
            };
        }
        if let Some(done) = current {
            push_block(&mut out, profile, table, done);
        }
    }
    out.unreadable.sort_unstable();
    out
}

fn push_block(out: &mut Plan, profile: &Profile, table: Table, (s, e, tags): (u32, u32, Vec<usize>)) {
    // s..=e fits in u16 and within the limit (checked by the caller).
    let start = s as u16;
    let quantity = (e - s + 1) as u16;
    match ReadRequest::for_table(table, start, quantity) {
        Ok(request) => out.blocks.push(ReadBlock {
            request,
            tags: tags
                .into_iter()
                .map(|i| {
                    let r = &profile.registers[i];
                    BlockTag { register: i, offset: r.address - start, width: r.width() }
                })
                .collect(),
        }),
        Err(_) => out.unreadable.extend(tags),
    }
}

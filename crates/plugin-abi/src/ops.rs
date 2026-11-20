//! Host op numbers, status codes, and the packed return values from `ABI.md` section 4.

use crate::permissions::PermissionKind;

/// A `craft_host.call` op. Numbers are stable. New ops are additive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Op {
    HostLog = 0x0001,
    HostInfo = 0x0002,
    HostCancelled = 0x0003,
    HostProgress = 0x0004,
    CmdList = 0x0010,
    CmdExecute = 0x0011,
    DocQuery = 0x0020,
    DocApply = 0x0021,
    EventPoll = 0x0030,
    LinkCall = 0x0040,
    LinkApps = 0x0041,
    StateGet = 0x0050,
    StatePut = 0x0051,
    FsOpenPicked = 0x0060,
    FsRead = 0x0061,
    FsWrite = 0x0062,
    NetFetch = 0x0070,
    TimeNow = 0x0080,
    RandomBytes = 0x0090,
}

impl Op {
    /// Every op in this release, in number order.
    pub const ALL: [Op; 19] = [
        Op::HostLog,
        Op::HostInfo,
        Op::HostCancelled,
        Op::HostProgress,
        Op::CmdList,
        Op::CmdExecute,
        Op::DocQuery,
        Op::DocApply,
        Op::EventPoll,
        Op::LinkCall,
        Op::LinkApps,
        Op::StateGet,
        Op::StatePut,
        Op::FsOpenPicked,
        Op::FsRead,
        Op::FsWrite,
        Op::NetFetch,
        Op::TimeNow,
        Op::RandomBytes,
    ];

    pub fn number(self) -> u32 {
        self as u32
    }

    /// `None` for numbers this release does not know, including the reserved 0x00A0 range.
    pub fn from_number(n: u32) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.number() == n)
    }

    /// The dotted name used in `requires.ops`.
    pub fn name(self) -> &'static str {
        match self {
            Op::HostLog => "host.log",
            Op::HostInfo => "host.info",
            Op::HostCancelled => "host.cancelled",
            Op::HostProgress => "host.progress",
            Op::CmdList => "cmd.list",
            Op::CmdExecute => "cmd.execute",
            Op::DocQuery => "doc.query",
            Op::DocApply => "doc.apply",
            Op::EventPoll => "event.poll",
            Op::LinkCall => "link.call",
            Op::LinkApps => "link.apps",
            Op::StateGet => "state.get",
            Op::StatePut => "state.put",
            Op::FsOpenPicked => "fs.open_picked",
            Op::FsRead => "fs.read",
            Op::FsWrite => "fs.write",
            Op::NetFetch => "net.fetch",
            Op::TimeNow => "time.now",
            Op::RandomBytes => "random.bytes",
        }
    }

    pub fn from_name(name: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.name() == name)
    }

    /// The permission kind the op needs. `None` means any plugin may call it.
    pub fn permission(self) -> Option<PermissionKind> {
        match self {
            Op::HostLog | Op::HostInfo | Op::HostCancelled | Op::HostProgress => None,
            Op::CmdList | Op::CmdExecute => Some(PermissionKind::CommandsRun),
            Op::DocQuery => Some(PermissionKind::DocRead),
            Op::DocApply => Some(PermissionKind::DocWrite),
            Op::EventPoll => Some(PermissionKind::CommandsObserve),
            Op::LinkCall | Op::LinkApps => Some(PermissionKind::LinkCall),
            Op::StateGet | Op::StatePut => Some(PermissionKind::StateLocal),
            Op::FsOpenPicked | Op::FsRead | Op::FsWrite => Some(PermissionKind::FsPicked),
            Op::NetFetch => Some(PermissionKind::NetFetch),
            Op::TimeNow => Some(PermissionKind::Clock),
            Op::RandomBytes => Some(PermissionKind::Random),
        }
    }

    /// True when a missing permission degrades the op instead of returning `E_DENIED`.
    /// `time.now` returns 0 and `random.bytes` returns a host-seeded deterministic stream.
    pub fn degrades_without_permission(self) -> bool {
        matches!(self, Op::TimeNow | Op::RandomBytes)
    }
}

/// Status in the low 32 bits of a `craft_host.call` return.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Status {
    Ok = 0,
    Denied = -1,
    TooSmall = -2,
    Unsupported = -3,
    BadArg = -4,
    Cancelled = -5,
    Limit = -6,
}

impl Status {
    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn from_code(code: i32) -> Option<Status> {
        match code {
            0 => Some(Status::Ok),
            -1 => Some(Status::Denied),
            -2 => Some(Status::TooSmall),
            -3 => Some(Status::Unsupported),
            -4 => Some(Status::BadArg),
            -5 => Some(Status::Cancelled),
            -6 => Some(Status::Limit),
            _ => None,
        }
    }

    /// The ABI name, such as `E_DENIED`. `Ok` is `OK`.
    pub fn name(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Denied => "E_DENIED",
            Status::TooSmall => "E_TOO_SMALL",
            Status::Unsupported => "E_UNSUPPORTED",
            Status::BadArg => "E_BAD_ARG",
            Status::Cancelled => "E_CANCELLED",
            Status::Limit => "E_LIMIT",
        }
    }
}

/// Pack a `craft_host.call` return: status in the low 32 bits, byte count in the high 32 bits.
pub fn pack_call_return(status: Status, count: u32) -> i64 {
    (i64::from(count) << 32) | i64::from(status.code() as u32)
}

/// Split a `craft_host.call` return. The status stays a raw code so a guest can see codes from a newer host.
pub fn unpack_call_return(value: i64) -> (i32, u32) {
    (value as i32, (value >> 32) as u32)
}

/// Pack a successful `craft_invoke` return: `len << 32 | ptr`. `None` when `len` needs bit 31,
/// because that would read back as a negative status.
pub fn pack_invoke_return(ptr: u32, len: u32) -> Option<i64> {
    if len > i32::MAX as u32 {
        return None;
    }
    Some((i64::from(len) << 32) | i64::from(ptr))
}

/// Split a `craft_invoke` return. A negative value is a status and carries no pointer.
pub fn unpack_invoke_return(value: i64) -> Result<(u32, u32), i64> {
    if value < 0 {
        return Err(value);
    }
    Ok((value as u32, (value >> 32) as u32))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn op_numbers_match_the_abi_table() {
        let table: [(Op, u32, &str); 19] = [
            (Op::HostLog, 0x0001, "host.log"),
            (Op::HostInfo, 0x0002, "host.info"),
            (Op::HostCancelled, 0x0003, "host.cancelled"),
            (Op::HostProgress, 0x0004, "host.progress"),
            (Op::CmdList, 0x0010, "cmd.list"),
            (Op::CmdExecute, 0x0011, "cmd.execute"),
            (Op::DocQuery, 0x0020, "doc.query"),
            (Op::DocApply, 0x0021, "doc.apply"),
            (Op::EventPoll, 0x0030, "event.poll"),
            (Op::LinkCall, 0x0040, "link.call"),
            (Op::LinkApps, 0x0041, "link.apps"),
            (Op::StateGet, 0x0050, "state.get"),
            (Op::StatePut, 0x0051, "state.put"),
            (Op::FsOpenPicked, 0x0060, "fs.open_picked"),
            (Op::FsRead, 0x0061, "fs.read"),
            (Op::FsWrite, 0x0062, "fs.write"),
            (Op::NetFetch, 0x0070, "net.fetch"),
            (Op::TimeNow, 0x0080, "time.now"),
            (Op::RandomBytes, 0x0090, "random.bytes"),
        ];
        assert_eq!(table.len(), Op::ALL.len());
        for (op, n, name) in table {
            assert_eq!(op.number(), n);
            assert_eq!(op.name(), name);
            assert_eq!(Op::from_number(n), Some(op));
            assert_eq!(Op::from_name(name), Some(op));
        }
    }

    #[test]
    fn unknown_and_reserved_ops_are_not_known() {
        assert_eq!(Op::from_number(0), None);
        assert_eq!(Op::from_number(0x00A0), None);
        assert_eq!(Op::from_number(0x00AF), None);
        assert_eq!(Op::from_name("frames.input_at"), None);
        assert_eq!(Op::from_name(""), None);
    }

    #[test]
    fn permission_mapping_follows_the_abi_table() {
        use PermissionKind as K;
        for op in [Op::HostLog, Op::HostInfo, Op::HostCancelled, Op::HostProgress] {
            assert_eq!(op.permission(), None);
        }
        assert_eq!(Op::CmdExecute.permission(), Some(K::CommandsRun));
        assert_eq!(Op::CmdList.permission(), Some(K::CommandsRun));
        assert_eq!(Op::DocQuery.permission(), Some(K::DocRead));
        assert_eq!(Op::DocApply.permission(), Some(K::DocWrite));
        assert_eq!(Op::EventPoll.permission(), Some(K::CommandsObserve));
        assert_eq!(Op::LinkCall.permission(), Some(K::LinkCall));
        assert_eq!(Op::LinkApps.permission(), Some(K::LinkCall));
        assert_eq!(Op::StatePut.permission(), Some(K::StateLocal));
        assert_eq!(Op::FsRead.permission(), Some(K::FsPicked));
        assert_eq!(Op::NetFetch.permission(), Some(K::NetFetch));
        assert_eq!(Op::TimeNow.permission(), Some(K::Clock));
        assert_eq!(Op::RandomBytes.permission(), Some(K::Random));
        let soft: Vec<Op> = Op::ALL.into_iter().filter(|o| o.degrades_without_permission()).collect();
        assert_eq!(soft, vec![Op::TimeNow, Op::RandomBytes]);
    }

    #[test]
    fn every_permissioned_op_kind_is_reachable_from_a_permission() {
        // Each op kind must be producible by Permission::kind, else the op could never be granted.
        use crate::permissions::Permission;
        let kinds: Vec<PermissionKind> = [
            "commands.run",
            "commands.observe",
            "doc.read",
            "doc.write",
            "state.local",
            "link.call:a",
            "fs.picked",
            "net.fetch:https://x",
            "clock",
            "random",
        ]
        .iter()
        .map(|s| s.parse::<Permission>().unwrap().kind())
        .collect();
        for op in Op::ALL {
            if let Some(k) = op.permission() {
                assert!(kinds.contains(&k), "{} has no granting permission", op.name());
            }
        }
    }

    #[test]
    fn status_codes_match_the_abi() {
        let table = [
            (Status::Ok, 0, "OK"),
            (Status::Denied, -1, "E_DENIED"),
            (Status::TooSmall, -2, "E_TOO_SMALL"),
            (Status::Unsupported, -3, "E_UNSUPPORTED"),
            (Status::BadArg, -4, "E_BAD_ARG"),
            (Status::Cancelled, -5, "E_CANCELLED"),
            (Status::Limit, -6, "E_LIMIT"),
        ];
        for (s, code, name) in table {
            assert_eq!(s.code(), code);
            assert_eq!(s.name(), name);
            assert_eq!(Status::from_code(code), Some(s));
        }
        assert_eq!(Status::from_code(1), None);
        assert_eq!(Status::from_code(-7), None);
    }

    #[test]
    fn call_return_packs_status_low_and_count_high() {
        let v = pack_call_return(Status::TooSmall, 4096);
        assert_eq!(v & 0xFFFF_FFFF, 0xFFFF_FFFE);
        assert_eq!(v >> 32, 4096);
        assert_eq!(unpack_call_return(v), (-2, 4096));
        assert_eq!(unpack_call_return(pack_call_return(Status::Ok, 7)), (0, 7));
        assert_eq!(unpack_call_return(pack_call_return(Status::Limit, u32::MAX)), (-6, u32::MAX));
        assert_eq!(pack_call_return(Status::Ok, 0), 0);
    }

    #[test]
    fn invoke_return_is_len_shl_32_or_ptr_and_negative_is_status() {
        let v = pack_invoke_return(0x1000, 12).unwrap();
        assert_eq!(v, (12i64 << 32) | 0x1000);
        assert_eq!(unpack_invoke_return(v), Ok((0x1000, 12)));
        assert_eq!(unpack_invoke_return(-3), Err(-3));
        assert_eq!(pack_invoke_return(u32::MAX, i32::MAX as u32).map(unpack_invoke_return), Some(Ok((u32::MAX, i32::MAX as u32))));
        assert_eq!(pack_invoke_return(0, 0x8000_0000), None);
    }
}

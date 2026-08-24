#[allow(non_upper_case_globals)]
impl Operation {
/// `ping` request.
pub const Ping: Self = Self::Protocol(Opcode::Ping);
/// `get` request.
pub const Get: Self = Self::Protocol(Opcode::Get);
/// `set` request.
pub const Set: Self = Self::Protocol(Opcode::Set);
/// `delete` request.
pub const Delete: Self = Self::Protocol(Opcode::Delete);
/// `experimental_stats` request.
pub const ExperimentalStats: Self = Self::Protocol(Opcode::ExperimentalStats);
/// `experimental_sync` request.
pub const ExperimentalSync: Self = Self::Protocol(Opcode::ExperimentalSync);
/// `namespace_open` request.
pub const NamespaceOpen: Self = Self::Protocol(Opcode::NamespaceOpen);
/// `namespace_update_policy` request.
pub const NamespaceUpdatePolicy: Self = Self::Protocol(Opcode::NamespaceUpdatePolicy);
/// `namespace_delete` request.
pub const NamespaceDelete: Self = Self::Protocol(Opcode::NamespaceDelete);
}

pub(crate) mod assistant;
pub(crate) mod control;
pub(crate) mod conversation;
// T6 wires the recorder into `/listen`; remove this allow then.
#[allow(dead_code)]
pub(crate) mod listening;
pub(crate) mod memory;
pub(crate) mod speech;

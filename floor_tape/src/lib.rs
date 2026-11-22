//! floor_tape as a library: the synthetic market and its books, with no server attached, so
//! the floor can run the same market inside a browser tab (`demo::InPageTape`). The `floor_tape`
//! binary (`src/main.rs`, feature `server`) serves them over the tape protocol.

pub mod demo;
pub mod l2;
pub mod rng;
pub mod sim;
pub mod store;
pub mod symbols;
pub mod synth;

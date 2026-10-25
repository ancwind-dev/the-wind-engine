//! Сеть: протокол, снимки с дельтами, сервер и клиент поверх UDP.
//! Как и почему так — заметка «Сеть — снимки и дельты».

pub mod bits;
pub mod client;
pub mod protocol;
pub mod replication;
pub mod server;
pub mod traffic;

pub use client::{ClientStatus, NetClient};
pub use protocol::DEFAULT_PORT;
pub use replication::{RemoteSoldier, RemoteWorld, ReplicationConfig};
pub use server::NetServer;
pub use traffic::Traffic;

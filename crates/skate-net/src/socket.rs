//! Datagram queues must hold a render frame's bulk traffic, including relay IPC.
use std::{io, net::UdpSocket};
pub fn configure(socket: &UdpSocket) -> io::Result<()> {
    let socket = socket2::SockRef::from(socket);
    socket.set_recv_buffer_size(4 * 1024 * 1024)?;
    socket.set_send_buffer_size(4 * 1024 * 1024)?;
    socket.set_nonblocking(true)
}

use anyhow::Error;
use embedded_io::{Read, Write};

pub struct YDlidar<UART> {
    uart: UART,
}

impl<UART> YDlidar<UART>
where
    UART: Read + Write,
{
    pub fn new(uart: UART) -> Result<Self, Error> {
        let lidar = YDlidar { uart };
        Ok(lidar)
    }
    pub fn send_command(&mut self, cmd: u8, payload: &[u8]) -> Result<(), UART::Error> {
        let mut checksum: u8 = cmd;

        self.uart.write(&[cmd])?;

        for &b in payload {
            checksum = checksum.wrapping_add(b);
            self.uart.write(&[b])?;
        }

        self.uart.write(&[checksum])?;
        Ok(())
    }

    pub fn read_response(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<(), embedded_io::ReadExactError<UART::Error>> {
        self.uart.read_exact(buffer)?;
        Ok(())
    }
}

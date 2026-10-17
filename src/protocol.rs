use serde_json::Value;
use std::io::{self, BufRead, Read, Write};

const MAX_MESSAGE: usize = 32 * 1024 * 1024;

pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let n = (&mut *reader).take(8193).read_line(&mut line)?;
        if n == 0 {
            return if header_bytes == 0 {
                Ok(None)
            } else {
                Err(invalid("Truncated header"))
            };
        }
        header_bytes += n;
        if header_bytes > 8192 {
            return Err(invalid("Header too large"));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("Malformed header"))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(invalid("Duplicate Content-Length"));
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| invalid("Invalid Content-Length"))?,
            );
        }
    }
    let length = length.ok_or_else(|| invalid("Missing Content-Length"))?;
    if length > MAX_MESSAGE {
        return Err(invalid("Message too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_message(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

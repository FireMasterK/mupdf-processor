use std::env;
use std::io;
use std::net::SocketAddr;
use std::num::NonZeroUsize;

use crate::upload::{MAX_ACCEPTED_UPLOAD_BYTES, MAX_IN_MEMORY_UPLOAD_BYTES};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub worker_count: usize,
    pub max_in_memory_upload_bytes: usize,
    pub max_accepted_upload_bytes: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, io::Error> {
        let bind_addr = parse_bind_addr(
            env::var("MUPDF_PROCESSOR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
        )?;

        let worker_count = env::var("MUPDF_PROCESSOR_CPUS")
            .ok()
            .map(|value| parse_usize_var("MUPDF_PROCESSOR_CPUS", &value))
            .transpose()?
            .unwrap_or_else(max_available_parallelism);

        let max_in_memory_upload_bytes = env::var("MUPDF_PROCESSOR_MAX_IN_MEMORY_BYTES")
            .ok()
            .map(|value| parse_usize_var("MUPDF_PROCESSOR_MAX_IN_MEMORY_BYTES", &value))
            .transpose()?
            .unwrap_or(MAX_IN_MEMORY_UPLOAD_BYTES);

        let max_accepted_upload_bytes = env::var("MUPDF_PROCESSOR_MAX_ACCEPTED_BYTES")
            .ok()
            .map(|value| parse_usize_var("MUPDF_PROCESSOR_MAX_ACCEPTED_BYTES", &value))
            .transpose()?
            .unwrap_or(MAX_ACCEPTED_UPLOAD_BYTES);

        Ok(Self {
            bind_addr,
            worker_count,
            max_in_memory_upload_bytes,
            max_accepted_upload_bytes,
        })
    }
}

fn parse_bind_addr(value: String) -> Result<SocketAddr, io::Error> {
    value.parse::<SocketAddr>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid MUPDF_PROCESSOR_BIND value `{value}`: {error}"),
        )
    })
}

fn parse_usize_var(name: &str, value: &str) -> Result<usize, io::Error> {
    value.parse::<usize>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {name} value `{value}`: {error}"),
        )
    })
}

pub fn max_available_parallelism() -> usize {
    std::thread::available_parallelism()
        .unwrap_or(NonZeroUsize::new(1).expect("nonzero"))
        .get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn max_parallelism_is_at_least_one() {
        assert!(max_available_parallelism() >= 1);
    }

    #[test]
    fn parse_bind_addr_works() {
        let addr = parse_bind_addr("127.0.0.1:9999".to_string()).expect("addr");
        assert_eq!(
            addr,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9999)
        );
    }
}

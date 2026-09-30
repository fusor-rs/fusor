use crate::{
    WorkerError,
    shared::{Codec, Payload},
};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Write};

pub(crate) const MESSAGE_LIMIT: usize = 16 * 1024 * 1024;

/// Values which can cross a worker boundary. Implementations are supplied by fusor.
pub trait Message: private::Sealed + Send + 'static {}
impl<T: Serialize + DeserializeOwned + Send + 'static> Message for T {}

pub(crate) mod private {
    use super::*;
    pub trait Sealed: Sized {
        fn encode(&self, limit: usize, codec: &Codec) -> Result<Payload, WorkerError>;
        fn decode(payload: Payload, codec: &Codec) -> Result<Self, WorkerError>;
    }
    impl<T: Serialize + DeserializeOwned + Send + 'static> Sealed for T {
        fn encode(&self, limit: usize, _: &Codec) -> Result<Payload, WorkerError> {
            Ok(Payload::Json(encode_json(self, limit)?))
        }
        fn decode(payload: Payload, codec: &Codec) -> Result<Self, WorkerError> {
            let Payload::Json(bytes) = payload else {
                codec.discard(payload);
                return Err(WorkerError::SharedTypeMismatch);
            };
            serde_json::from_str(&bytes).map_err(|error| WorkerError::Decode {
                message: error.to_string(),
            })
        }
    }
}
pub(crate) fn encode_json(value: &impl Serialize, limit: usize) -> Result<String, WorkerError> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        limit,
        actual: 0,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if writer.actual > limit {
            WorkerError::PayloadTooLarge {
                limit,
                actual: writer.actual,
            }
        } else {
            WorkerError::Encode {
                message: error.to_string(),
            }
        }
    })?;
    Ok(String::from_utf8(writer.bytes).expect("JSON is UTF-8"))
}
struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
    actual: usize,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.actual = self.actual.saturating_add(bytes.len());
        if self.actual > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "worker payload limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{private::Sealed, *};
    #[test]
    fn encoding_stops_at_the_transport_bound() {
        struct Unbounded;
        impl Serialize for Unbounded {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(None)?;
                loop {
                    seq.serialize_element(&0_u8)?;
                }
            }
        }
        let mut writer = LimitedWriter {
            bytes: Vec::new(),
            limit: 32,
            actual: 0,
        };
        assert!(serde_json::to_writer(&mut writer, &Unbounded).is_err());
        assert!(writer.bytes.len() <= 32);
        assert!(matches!(
            "too long".to_owned().encode(3, &Codec::default()),
            Err(WorkerError::PayloadTooLarge { limit: 3, .. })
        ));
        assert_eq!(
            String::decode(
                "é".to_owned().encode(4, &Codec::default()).unwrap(),
                &Codec::default()
            )
            .unwrap(),
            "é"
        );
    }
}

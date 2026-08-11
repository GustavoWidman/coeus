use serde::{
    Deserializer, Serializer,
    de::{Error, Visitor},
};
use std::fmt;

pub fn serialize<S>(key: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(
        &key.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: Deserializer<'de>,
{
    struct PskVisitor;

    impl<'de> Visitor<'de> for PskVisitor {
        type Value = [u8; 32];

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("exactly 64 hexadecimal characters")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            if value.len() != 64 || !value.is_ascii() {
                return Err(E::custom("PSK must be exactly 64 hex characters"));
            }

            let mut key = [0u8; 32];

            let (chunks, remainder) = value.as_bytes().as_chunks::<2>();

            if !remainder.is_empty() {
                unreachable!(
                    "value length is guaranteed to be 64, so there should be no remainder"
                );
            }

            for (index, pair) in chunks.iter().enumerate() {
                key[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
            }

            Ok(key)
        }
    }

    fn hex<E>(byte: u8) -> Result<u8, E>
    where
        E: Error,
    {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            b'A'..=b'F' => Ok(byte - b'A' + 10),
            _ => Err(E::custom("PSK contains non-hex characters")),
        }
    }

    deserializer.deserialize_str(PskVisitor)
}

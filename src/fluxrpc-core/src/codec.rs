pub trait Codec: Clone + Sync + Send + 'static {
    fn encode<T: serde::Serialize>(&self, value: &T) -> anyhow::Result<Vec<u8>>;
    fn decode<T: serde::de::DeserializeOwned>(&self, data: &[u8]) -> anyhow::Result<T>;
}

pub mod json {
    use crate::codec::Codec;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    #[derive(Clone)]
    pub struct JsonCodec;

    impl JsonCodec {
        pub fn new() -> Self {
            Self {}
        }
    }

    impl Codec for JsonCodec {
        fn encode<T: Serialize>(&self, value: &T) -> anyhow::Result<Vec<u8>> {
            let json = serde_json::to_vec(value)?;
            Ok(json)
        }

        fn decode<T: DeserializeOwned>(&self, data: &[u8]) -> anyhow::Result<T> {
            let val = serde_json::from_slice(data)?;
            Ok(val)
        }
    }
}

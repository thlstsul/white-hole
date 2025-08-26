#[macro_export]
macro_rules! impl_serialize {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl serde::Serialize for $ty {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: serde::Serializer,
                {
                    serializer.serialize_str(&self.to_string())
                }
            }
        )+
    };
}

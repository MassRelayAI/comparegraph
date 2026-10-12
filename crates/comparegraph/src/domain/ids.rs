use std::fmt;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $name(pub i64);
        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                Self(v)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}
id_type!(RepoId);
id_type!(RunId);
id_type!(NodeId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_display_and_convert() {
        let r: RepoId = 7.into();
        assert_eq!(r.to_string(), "7");
        assert_eq!(RunId(3), RunId::from(3));
    }
}

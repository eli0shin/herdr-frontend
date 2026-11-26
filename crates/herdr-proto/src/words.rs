//! The protocol's string enums. Each keeps a word it doesn't know as `Other`, so a newer core's
//! words survive a decode and an encode unchanged.

/// Defines a string enum with an `Other(String)` fallback, serialized as its word.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $word:literal, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
            /// A word this crate doesn't know, kept as the core sent it.
            Other(String),
        }

        impl $name {
            /// The word on the wire.
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $word, )*
                    Self::Other(word) => word,
                }
            }
        }

        impl From<&str> for $name {
            fn from(word: &str) -> Self {
                match word {
                    $( $word => Self::$variant, )*
                    other => Self::Other(other.to_owned()),
                }
            }
        }

        impl From<String> for $name {
            fn from(word: String) -> Self {
                Self::from(word.as_str())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)
                    .map(|word| Self::from(word.as_ref()))
            }
        }
    };
}

pub(crate) use string_enum;

#[cfg(test)]
mod tests {
    string_enum! {
        /// A test enum.
        pub enum Word {
            One = "one",
            Two = "two",
        }
    }

    #[test]
    fn known_words_parse_and_unknown_words_survive() {
        assert_eq!(Word::from("one"), Word::One);
        assert_eq!(Word::from("three"), Word::Other("three".into()));
        assert_eq!(Word::Other("three".into()).as_str(), "three");
        assert_eq!(Word::Two.to_string(), "two");
        let words: Vec<Word> = serde_json::from_str(r#"["one","two","three"]"#).unwrap();
        assert_eq!(words, [Word::One, Word::Two, Word::Other("three".into())]);
        assert_eq!(
            serde_json::to_string(&words).unwrap(),
            r#"["one","two","three"]"#
        );
    }

    #[test]
    fn escaped_words_decode() {
        let word: Word = serde_json::from_str(r#""o\u006ee""#).unwrap();
        assert_eq!(word, Word::One);
    }
}

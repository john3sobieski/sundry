//! In-memory data passed into and out of tools (no file paths).

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    Empty,
    Text(String),
    Bytes(Vec<u8>),
}

impl Data {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Data::Empty => &[],
            Data::Text(s) => s.as_bytes(),
            Data::Bytes(b) => b,
        }
    }

    /// Text view; byte data must be valid UTF-8.
    pub fn as_text(&self) -> Result<&str> {
        match self {
            Data::Empty => Ok(""),
            Data::Text(s) => Ok(s),
            Data::Bytes(b) => std::str::from_utf8(b)
                .map_err(|e| Error::InvalidInput(format!("not valid UTF-8: {e}"))),
        }
    }
}

/// One piece of data with an optional name hint (e.g. the original file name).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: Option<String>,
    pub data: Data,
}

impl Item {
    pub fn empty() -> Self {
        Self { name: None, data: Data::Empty }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self { name: None, data: Data::Text(text.into()) }
    }

    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self { name: None, data: Data::Bytes(bytes.into()) }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Input {
    pub items: Vec<Item>,
}

impl Input {
    pub fn single(item: Item) -> Self {
        Self { items: vec![item] }
    }

    /// First item, or an error if the input is empty.
    pub fn first(&self) -> Result<&Item> {
        self.items
            .first()
            .ok_or_else(|| Error::InvalidInput("no input items".to_owned()))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output {
    pub items: Vec<Item>,
}

impl Output {
    pub fn single(item: Item) -> Self {
        Self { items: vec![item] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_view_of_each_variant() {
        assert_eq!(Data::Empty.as_text().unwrap(), "");
        assert_eq!(Data::Text("hi".into()).as_text().unwrap(), "hi");
        assert_eq!(Data::Bytes(b"yo".to_vec()).as_text().unwrap(), "yo");
        assert!(matches!(
            Data::Bytes(vec![0xff, 0xfe]).as_text(),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn bytes_view_of_each_variant() {
        assert!(Data::Empty.as_bytes().is_empty());
        assert_eq!(Data::Text("ab".into()).as_bytes(), b"ab");
    }

    #[test]
    fn first_fails_on_empty_input() {
        assert!(Input::default().first().is_err());
        let input = Input::single(Item::text("x").named("a.txt"));
        assert_eq!(input.first().unwrap().name.as_deref(), Some("a.txt"));
    }
}

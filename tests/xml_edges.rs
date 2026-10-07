//! Edge-of-port error arms for the XML reader: refusals the monorepo's
//! ported suite reaches only in passing, pinned here by name so coverage
//! and behaviour never regress. Every test asserts the exact
//! [`Error`] variant and payload the reader documents.

use pith_digest::Error;
use pith_zip::{XmlEvent, XmlReader};

/// Drives `input` to completion, returning the first error it produces.
fn first_error(input: &str) -> Error {
    let mut reader = XmlReader::new(input);
    loop {
        match reader.next() {
            Some(Ok(XmlEvent::Text(_))) | Some(Ok(_)) => continue,
            Some(Err(e)) => return e,
            None => panic!("input {input:?} produced no error"),
        }
    }
}

/// An unterminated CDATA section is a truncation, not a silent text node.
#[test]
fn unterminated_cdata_is_truncated() {
    assert_eq!(
        first_error("<a><![CDATA[never ends"),
        Error::truncated("xml cdata", 13, 10)
    );
}

/// An end tag cut off before its `>` is a truncation.
#[test]
fn truncated_end_tag_is_truncated() {
    assert_eq!(first_error("<a></a"), Error::truncated("xml end tag", 1, 0));
}

/// A self-closing tag cut off after the `/` is a truncation.
#[test]
fn truncated_self_closing_tag_is_truncated() {
    assert_eq!(first_error("<a/"), Error::truncated("xml tag", 1, 0));
}

/// A character after `/` that is not `>` is a named bad value.
#[test]
fn junk_after_self_closing_slash_is_bad_value() {
    assert_eq!(first_error("<a/b"), Error::BadValue("xml self-closing tag"));
}

/// An attribute whose `=` is missing is a named bad value; one cut off
/// before it is a truncation.
#[test]
fn attribute_without_equals_is_bad_value() {
    assert_eq!(first_error("<a b x/>"), Error::BadValue("xml attribute"));
}

#[test]
fn attribute_cut_before_equals_is_truncated() {
    assert_eq!(first_error("<a b"), Error::truncated("xml attribute", 1, 0));
}

/// A quote character that is neither `"` nor `'` is a named bad value.
#[test]
fn unquoted_attribute_value_is_bad_value() {
    assert_eq!(
        first_error("<a b=c/>"),
        Error::BadValue("xml attribute quote")
    );
}

/// An attribute value cut off before its closing quote is a truncation.
#[test]
fn unterminated_attribute_value_is_truncated() {
    assert_eq!(
        first_error("<a b=\"unclosed/>"),
        Error::truncated("xml attribute value", 11, 10)
    );
}

/// A `<` inside an attribute value is refused by name.
#[test]
fn less_than_in_attribute_value_is_bad_value() {
    assert_eq!(
        first_error("<a b=\"<"),
        Error::BadValue("xml '<' in attribute value")
    );
}

/// An entity body holding whitespace (or `<`/`&`) is refused by name.
#[test]
fn whitespace_entity_body_is_bad_value() {
    assert_eq!(first_error("<a>& ;</a>"), Error::BadValue("xml entity"));
}

/// A name position that hits end-of-input is a truncation; one that hits
/// a non-name character is a bad value.
#[test]
fn empty_name_position_is_truncated() {
    assert_eq!(
        first_error("<a></"),
        Error::truncated("xml end tag name", 1, 0)
    );
}

#[test]
fn non_name_character_is_bad_value() {
    assert_eq!(first_error("< </a>"), Error::BadValue("xml tag name"));
}

/// Unterminated skipped constructs each carry their own truncation name.
#[test]
fn unterminated_comment_is_truncated() {
    assert_eq!(
        first_error("<a><!-- forever"),
        Error::truncated("xml comment", 11, 8)
    );
}

#[test]
fn unterminated_processing_instruction_is_truncated() {
    assert_eq!(
        first_error("<?xml version=\"1.0\""),
        Error::truncated("xml processing instruction", 19, 17)
    );
}

#[test]
fn unterminated_declaration_is_truncated() {
    assert_eq!(
        first_error("<!DOCTYPE note ["),
        Error::truncated("xml declaration", 15, 14)
    );
}

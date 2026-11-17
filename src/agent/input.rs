//! A message from the user: text with `[Image #N]` placeholders and the
//! images they stand for (RFC 0034).

use serde::{Deserialize, Serialize};

use crate::image::{Numbered, placeholder};
use crate::provider::Block;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Saved")]
pub struct UserInput {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Numbered>,
}

/// A queued message as `session.json` holds it: text alone from an older
/// `codeagent`, or text and images.
#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    Text(String),
    Full {
        text: String,
        #[serde(default)]
        images: Vec<Numbered>,
    },
}

impl From<Saved> for UserInput {
    fn from(saved: Saved) -> UserInput {
        match saved {
            Saved::Text(text) => UserInput::from(text),
            Saved::Full { text, images } => UserInput { text, images },
        }
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> UserInput {
        UserInput {
            text,
            images: Vec::new(),
        }
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> UserInput {
        UserInput::from(text.to_owned())
    }
}

impl UserInput {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.images.is_empty()
    }

    /// The blocks of the message, in text order: each image follows the
    /// text up to and with its placeholder, so the model can refer to
    /// "image 1". An image whose placeholder is not in the text comes at
    /// the end.
    pub fn blocks(&self) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut rest = self.text.as_str();
        let mut later = Vec::new();
        let push_text = |blocks: &mut Vec<Block>, text: &str| {
            if !text.trim().is_empty() {
                blocks.push(Block::Text {
                    text: text.to_owned(),
                });
            }
        };
        for numbered in &self.images {
            let mark = placeholder(numbered.number);
            let split = rest.find(&mark).and_then(|at| {
                let end = at + mark.len();
                Some((rest.get(..end)?, rest.get(end..)?))
            });
            match split {
                Some((before, after)) => {
                    push_text(&mut blocks, before);
                    blocks.push(Block::Image(numbered.image.clone()));
                    rest = after;
                }
                None => later.push(Block::Image(numbered.image.clone())),
            }
        }
        push_text(&mut blocks, rest);
        blocks.extend(later);
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{Image, fixtures};

    fn image(number: u32, width: u32) -> Numbered {
        Numbered {
            number,
            image: Image::from_bytes(fixtures::png(width, 1)).unwrap(),
        }
    }

    fn shape(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Text { text } => format!("text {text:?}"),
                Block::Image(i) => format!("image {}", i.width),
                _ => "other".to_owned(),
            })
            .collect()
    }

    #[test]
    fn blocks_in_text_order() {
        let input = UserInput {
            text: "compare [Image #1] with [Image #3] please".into(),
            images: vec![image(1, 10), image(3, 30)],
        };
        assert_eq!(
            shape(&input.blocks()),
            [
                "text \"compare [Image #1]\"",
                "image 10",
                "text \" with [Image #3]\"",
                "image 30",
                "text \" please\""
            ]
        );
    }

    #[test]
    fn images_alone_and_without_placeholder() {
        let input = UserInput {
            text: "[Image #1]".into(),
            images: vec![image(1, 10), image(2, 20)],
        };
        assert_eq!(
            shape(&input.blocks()),
            ["text \"[Image #1]\"", "image 10", "image 20"]
        );
        assert!(!input.is_empty());
        assert!(UserInput::from(" ").is_empty());
    }

    #[test]
    fn queued_text_of_an_older_codeagent_reads() {
        let old: UserInput = serde_json::from_str("\"hi\"").unwrap();
        assert_eq!(old, UserInput::from("hi"));
        let input = UserInput {
            text: "a [Image #1]".into(),
            images: vec![image(1, 10)],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(!json.contains("\"data\""), "{json}");
        let back: UserInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.images[0].image.hash, input.images[0].image.hash);
        assert!(!back.images[0].image.loaded());
        let plain = serde_json::to_string(&UserInput::from("x")).unwrap();
        assert_eq!(plain, r#"{"text":"x"}"#);
    }
}

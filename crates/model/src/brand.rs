//! A project's brand: the colours, the face and the mark its ready-made pieces are made in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the ready-made pieces (`promo.add`) take their look from, unless they are told otherwise.
/// Setting it changes pieces added afterwards, not the ones already on the timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Brand {
    /// The main colour, straight sRGB: buttons, tags, badges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<[f32; 4]>,
    /// The colour beside it: highlights, the better of two prices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<[f32; 4]>,
    /// The face titles are set in: a font among the project's media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The mark: an image among the project's media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

impl Brand {
    /// Nothing set.
    pub fn is_empty(&self) -> bool {
        *self == Brand::default()
    }

    /// What is wrong with its colours, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        let colours = self.primary.iter().chain(self.secondary.iter());
        colours.flatten().any(|c| !(0.0..=1.0).contains(c)).then_some("colours are 0 to 1")
    }
}

/// A ready-made piece as it was made: which one, and everything it was told or took from the
/// brand at the time. Kept by the timeline of a compound clip that is one, so that the piece can
/// be made again with other words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Made {
    /// Which piece (`promo.list`).
    pub kind: String,
    /// Its words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Its further lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// The number it counts to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
    /// Its further numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f32>,
    /// Where its counting starts.
    #[serde(default)]
    pub from: f32,
    /// Its main colour and the one beside it.
    pub color: [f32; 4],
    /// The colour beside it.
    pub color2: [f32; 4],
    /// The way it leans, points or moves, if it was told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Where it sits in the frame, as shares of its size from the centre, and its size.
    pub x: f32,
    /// Vertical offset.
    pub y: f32,
    /// Size; 1 is as designed.
    pub scale: f32,
    /// How long it was made to show.
    pub duration: reelforge_time::Tick,
    /// The font its words are set in and the logo it shows: media of the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// A picture it shows (a product card's product): an image or video of the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Where its words and numbers come from, for a piece written from the project's copy.
    #[serde(default, skip_serializing_if = "Forms::is_empty")]
    pub forms: Forms,
}

/// How a piece's words and numbers are written from the project's copy ([`CopyField`]): each is
/// what the piece would be told, with `{name}` where the words of the field called `name` go. So
/// a row `✓ {point1}` follows the field `point1`, and a value `{price}` counts to the field
/// `price`. What is `None` was told to the piece as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Forms {
    /// Its words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Its lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<String>>,
    /// The number it counts to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Its further numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

impl Forms {
    /// Written from nothing.
    pub fn is_empty(&self) -> bool {
        *self == Forms::default()
    }

    /// Every form there is.
    pub fn all(&self) -> impl Iterator<Item = &str> {
        let one = self.text.iter().chain(&self.value);
        let many = self.lines.iter().flatten().chain(self.values.iter().flatten());
        one.chain(many).map(String::as_str)
    }
}

/// The fields `form` names: what stands between braces, where it could be a field's name.
pub fn fields_in(form: &str) -> impl Iterator<Item = &str> {
    form.split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}'))
        .map(|(name, _)| name)
        .filter(|name| is_field_name(name))
}

/// Whether `name` can name a field of the copy: letters and digits, starting with a letter, and
/// not `n`, which is where a counter's number goes.
pub fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    name != "n"
        && name.len() <= 40
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// `form` with the words of each field it names in their place; a name `copy` does not know
/// stays as it is written.
pub fn written(form: &str, copy: &[CopyField]) -> String {
    let mut out = String::with_capacity(form.len());
    let mut rest = form;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let known = after
            .split_once('}')
            .and_then(|(name, tail)| Some((copy.iter().find(|f| is_field_name(name) && f.name == name)?, tail)));
        match known {
            Some((field, tail)) => {
                out.push_str(&field.value);
                rest = tail;
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A named piece of a project's copy: the product's name, the hook, the price. Ready-made pieces
/// written from it ([`Forms`]) are made again when it is given other words (`storyboard.fill`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CopyField {
    /// Its name, as forms write it between braces.
    pub name: String,
    /// Its words; a number is written as it is said.
    pub value: String,
}

/// A caption written from a project's copy: `storyboard.fill` writes it again when a field it
/// names is given other words, for as long as nobody has changed it by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CopyLine {
    /// The title clip it is.
    pub clip: String,
    /// What it is written from, fields between braces.
    pub form: String,
    /// What it said when it was last written.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_written_from_the_copy_by_name() {
        let copy = [
            CopyField { name: "product".into(), value: "保温杯".into() },
            CopyField { name: "price".into(), value: "129".into() },
        ];
        assert_eq!(written("{product}，到手 ¥{price}", &copy), "保温杯，到手 ¥129");
        // a counter's place and a name nobody knows are left alone
        assert_eq!(written("¥{n} · {shop} {", &copy), "¥{n} · {shop} {");
        assert_eq!(written("{price}", &copy).parse::<f32>(), Ok(129.0));
        assert_eq!(fields_in("{product}，¥{n}，{price}{}").collect::<Vec<_>>(), ["product", "price"]);
        assert!(is_field_name("point1") && !is_field_name("n") && !is_field_name("1st") && !is_field_name("a b"));
        let forms = Forms { text: Some("{product}".into()), lines: Some(vec!["✓ {a}".into()]), ..Forms::default() };
        assert_eq!(forms.all().collect::<Vec<_>>(), ["{product}", "✓ {a}"]);
        assert!(Forms::default().is_empty() && !forms.is_empty());
    }
}

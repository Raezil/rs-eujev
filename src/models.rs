use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::DEFAULT_MODEL;

/// A named question. Constructors add the JSON `type` discriminator automatically.
/// Instructions and descriptions may contain arbitrary JSON; constraints are
/// validated by the service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Select one of two to ten named options.
    Choice {
        /// The question to evaluate.
        instructions: Value,
        /// Option names mapped to descriptions; null descriptions use the name.
        criteria: BTreeMap<String, Value>,
    },
    /// Estimate the probability that a condition is true.
    Noul {
        /// The condition to evaluate.
        instructions: Value,
        /// Optional descriptions for `true` and `false`.
        #[serde(
            default,
            skip_serializing_if = "BTreeMap::is_empty",
            deserialize_with = "null_default"
        )]
        criteria: BTreeMap<String, Value>,
    },
    /// Estimate a position on an ordered scale of two to ten levels.
    Score {
        /// The question to evaluate.
        instructions: Value,
        /// Level descriptions ordered from low to high.
        criteria: Vec<Value>,
    },
}

impl Question {
    /// Construct a choice question from named options.
    pub fn choice<K, V>(
        instructions: impl Into<Value>,
        criteria: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        K: Into<String>,
        V: Into<Value>,
    {
        Self::Choice {
            instructions: instructions.into(),
            criteria: criteria
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Construct a probability question without optional criteria.
    pub fn noul(instructions: impl Into<Value>) -> Self {
        Self::Noul {
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
        }
    }

    /// Construct a probability question with descriptions for `true` and `false`.
    pub fn noul_with_criteria<K, V>(
        instructions: impl Into<Value>,
        criteria: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        K: Into<String>,
        V: Into<Value>,
    {
        Self::Noul {
            instructions: instructions.into(),
            criteria: criteria
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Construct a score question from ordered level descriptions.
    pub fn score<V: Into<Value>>(
        instructions: impl Into<Value>,
        criteria: impl IntoIterator<Item = V>,
    ) -> Self {
        Self::Score {
            instructions: instructions.into(),
            criteria: criteria.into_iter().map(Into::into).collect(),
        }
    }
}

/// Request for `POST /v1/systemone`. The service validates counts and limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    /// Model name; an empty string serializes as [`DEFAULT_MODEL`].
    #[serde(default = "default_model", serialize_with = "serialize_model")]
    pub model: String,
    /// State to evaluate: normally a string, object, or array.
    pub state: Value,
    /// One to eight questions, keyed by name.
    pub questions: BTreeMap<String, Question>,
}

impl DecisionRequest {
    /// Create a request using the default model, then add named questions.
    pub fn new(state: impl Into<Value>) -> Self {
        Self {
            model: default_model(),
            state: state.into(),
            questions: BTreeMap::new(),
        }
    }

    /// Select a model, for example `jeff-1.0.0`.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Add a named question, replacing an existing question with the same name.
    pub fn with_question(mut self, name: impl Into<String>, question: Question) -> Self {
        self.questions.insert(name.into(), question);
        self
    }
}

fn default_model() -> String {
    DEFAULT_MODEL.into()
}

fn serialize_model<S: Serializer>(model: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    })
}

/// An answer discriminator. Unknown strings are retained for forward compatibility.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum QuestionType {
    /// Named option selection.
    Choice,
    /// Probability of a condition being true.
    Noul,
    /// Expected position on an ordered scale.
    Score,
    /// A type added by the service that this SDK does not yet recognize.
    Unknown(String),
}

impl QuestionType {
    /// Return the original JSON type string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Choice => "choice",
            Self::Noul => "noul",
            Self::Score => "score",
            Self::Unknown(value) => value,
        }
    }
}

impl Default for QuestionType {
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl From<String> for QuestionType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "choice" => Self::Choice,
            "noul" => Self::Noul,
            "score" => Self::Score,
            _ => Self::Unknown(value),
        }
    }
}

impl From<QuestionType> for String {
    fn from(value: QuestionType) -> Self {
        value.as_str().into()
    }
}

impl fmt::Display for QuestionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An answer to a named question. Check `question_type` before reading its fields.
/// Optional numeric values distinguish an absent value from a valid zero.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    /// Discriminator, serialized as `type`.
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub question_type: QuestionType,
    /// Selected option for a choice question.
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub choice: String,
    /// Probability that the condition is true (0 to 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noul: Option<f64>,
    /// Expected position on the scale (0 to N−1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Confidence for choice and score questions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Probability per option or scale position.
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "null_default"
    )]
    pub probabilities: BTreeMap<String, f64>,
    /// Scale positions mapped to their descriptions.
    #[serde(
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "null_default"
    )]
    pub legend: BTreeMap<String, String>,
}

/// Token accounting returned by the service.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Number of input tokens.
    #[serde(deserialize_with = "null_default")]
    pub input_tokens: u64,
    /// Number of output tokens.
    #[serde(deserialize_with = "null_default")]
    pub output_tokens: u64,
}

/// Server processing details and the exact cost.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    /// Request identifier; the client falls back to the `X-Request-ID` header.
    #[serde(deserialize_with = "null_default")]
    pub request_id: String,
    /// Server mode, normally `live`.
    #[serde(deserialize_with = "null_default")]
    pub mode: String,
    /// Server-side latency in milliseconds.
    #[serde(deserialize_with = "null_default")]
    pub latency_ms: i64,
    /// Exact decimal cost in euros. Kept as a string to avoid rounding.
    #[serde(
        skip_serializing_if = "String::is_empty",
        deserialize_with = "null_default"
    )]
    pub cost_eur: String,
}

/// Named answers and request accounting. Unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DecisionResponse {
    /// The model release that answered.
    pub model: String,
    /// Answers under the same names as the input questions.
    pub answers: BTreeMap<String, Answer>,
    /// Token accounting.
    pub usage: Usage,
    /// Request metadata.
    pub meta: Metadata,
}

impl<'de> Deserialize<'de> for DecisionResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // A map is required even when all fields are absent. This rejects `null`
        // and arrays that a derived struct deserializer might otherwise accept.
        let map = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Fields {
            #[serde(deserialize_with = "null_default")]
            model: String,
            #[serde(deserialize_with = "answer_map")]
            answers: BTreeMap<String, Answer>,
            #[serde(deserialize_with = "object_or_default")]
            usage: Usage,
            #[serde(deserialize_with = "object_or_default")]
            meta: Metadata,
        }
        let fields: Fields =
            serde_json::from_value(Value::Object(map)).map_err(serde::de::Error::custom)?;
        Ok(Self {
            model: fields.model,
            answers: fields.answers,
            usage: fields.usage,
            meta: fields.meta,
        })
    }
}

pub(crate) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn object_or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let map = Option::<serde_json::Map<String, Value>>::deserialize(deserializer)?;
    match map {
        Some(map) => serde_json::from_value(Value::Object(map)).map_err(serde::de::Error::custom),
        None => Ok(T::default()),
    }
}

fn answer_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Answer>, D::Error> {
    let maps = Option::<BTreeMap<String, Option<serde_json::Map<String, Value>>>>::deserialize(
        deserializer,
    )?;
    maps.unwrap_or_default()
        .into_iter()
        .map(|(name, map)| {
            let answer = serde_json::from_value(Value::Object(map.unwrap_or_default()))
                .map_err(serde::de::Error::custom)?;
            Ok((name, answer))
        })
        .collect()
}

use eujev::{json, DecisionRequest, DecisionResponse, Question, QuestionType, DEFAULT_MODEL};

#[test]
fn all_question_wire_formats() {
    let request = DecisionRequest::new(json!({"message": "Hello", "history": []}))
        .with_model("jeff-1.0.0")
        .with_question(
            "team",
            Question::choice(
                json!({"ask": "Which team?"}),
                [
                    ("billing", json!(null)),
                    ("support", json!(["Technical", "Bugs"])),
                ],
            ),
        )
        .with_question("urgent", Question::noul("Is this urgent?"))
        .with_question(
            "refund",
            Question::noul_with_criteria(
                "Refund?",
                [
                    ("true", json!({"reason": "Duplicate charge"})),
                    ("false", json!("Valid charge")),
                ],
            ),
        )
        .with_question(
            "severity",
            Question::score(json!(null), [json!(null), json!({"level": "high"})]),
        );
    let expected = json!({
        "model": "jeff-1.0.0", "state": {"message": "Hello", "history": []},
        "questions": {
            "team": {"type": "choice", "instructions": {"ask": "Which team?"}, "criteria": {"billing": null, "support": ["Technical", "Bugs"]}},
            "urgent": {"type": "noul", "instructions": "Is this urgent?"},
            "refund": {"type": "noul", "instructions": "Refund?", "criteria": {"true": {"reason": "Duplicate charge"}, "false": "Valid charge"}},
            "severity": {"type": "score", "instructions": null, "criteria": [null, {"level": "high"}]}
        }
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<DecisionRequest>(expected).unwrap(),
        request
    );
}

#[test]
fn empty_model_defaults_without_mutation_and_empty_noul_criteria_are_omitted() {
    let request = DecisionRequest::new(json!(["first", "second"]))
        .with_model("")
        .with_question(
            "q",
            Question::Noul {
                instructions: json!(null),
                criteria: Default::default(),
            },
        );
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["model"], DEFAULT_MODEL);
    assert!(request.model.is_empty());
    assert!(value["questions"]["q"].get("criteria").is_none());
    let decoded: DecisionRequest =
        serde_json::from_value(json!({"state": "hi", "questions": {}})).unwrap();
    assert_eq!(decoded.model, DEFAULT_MODEL);
}

#[test]
fn responses_preserve_zero_unknown_types_and_exact_cost() {
    let value = json!({
        "model": "jeff-1.0.0",
        "answers": {
            "urgent": {"type": "noul", "noul": 0},
            "severity": {"type": "score", "score": 0, "confidence": 0, "probabilities": {"0": 1, "1": 0}, "legend": {"0": "low", "1": "high"}},
            "future": {"type": "new-answer-type", "new_field": true}
        },
        "usage": {"input_tokens": 64, "output_tokens": 0},
        "meta": {"cost_eur": "0.0000023680000000001", "new_field": true},
        "new_field": true
    });
    let response: DecisionResponse = serde_json::from_value(value).unwrap();
    assert_eq!(response.answers["urgent"].question_type, QuestionType::Noul);
    assert_eq!(response.answers["urgent"].noul, Some(0.0));
    assert_eq!(response.answers["urgent"].score, None);
    assert_eq!(response.answers["severity"].score, Some(0.0));
    assert_eq!(response.answers["severity"].confidence, Some(0.0));
    assert_eq!(response.answers["severity"].legend["1"], "high");
    assert_eq!(
        response.answers["future"].question_type,
        QuestionType::Unknown("new-answer-type".into())
    );
    assert_eq!(response.meta.cost_eur, "0.0000023680000000001");
    assert_eq!(
        serde_json::to_value(&response.answers["urgent"]).unwrap(),
        json!({"type": "noul", "noul": 0.0})
    );
    assert_eq!(
        serde_json::from_value::<DecisionResponse>(serde_json::to_value(&response).unwrap())
            .unwrap(),
        response
    );
}

#[test]
fn missing_and_nullable_response_fields_have_defaults() {
    for value in [
        json!({}),
        json!({"model": null, "answers": null, "usage": null, "meta": null}),
    ] {
        assert_eq!(
            serde_json::from_value::<DecisionResponse>(value).unwrap(),
            DecisionResponse::default()
        );
    }
    let response: DecisionResponse = serde_json::from_value(json!({
        "answers": {"empty": null, "n": {"type": null, "noul": null, "choice": null}},
        "usage": {"input_tokens": null}, "meta": {"request_id": null}
    }))
    .unwrap();
    assert_eq!(response.answers["empty"], Default::default());
    assert_eq!(response.answers["n"], Default::default());
}

#[test]
fn invalid_response_shapes_are_rejected() {
    for invalid in [
        "null",
        "[]",
        "[null,null,null,null]",
        "true",
        "\"text\"",
        "{} {}",
        r#"{"answers":[]}"#,
        r#"{"answers":{"q":[]}}"#,
        r#"{"usage":[]}"#,
        r#"{"meta":[]}"#,
        r#"{"model":42}"#,
        r#"{"meta":{"cost_eur":0.02}}"#,
        r#"{"usage":{"input_tokens":-1}}"#,
        r#"{"usage":{"input_tokens":1.5}}"#,
        r#"{"answers":{"q":{"noul":true}}}"#,
        r#"{"answers":{"q":{"score":1e400}}}"#,
        r#"{"answers":{"q":{"probabilities":{"a":"bad"}}}}"#,
    ] {
        assert!(
            serde_json::from_str::<DecisionResponse>(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

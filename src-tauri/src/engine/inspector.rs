//! Request requirement inspection and model capability gating.

use serde_json::Value;

use crate::providers;

/// What an incoming request actually requires, so members that can't supply it
/// are skipped instead of being tried and failing (or worse, not failing).
pub(crate) struct Needs {
    pub(crate) vision: bool,
    pub(crate) tools: bool,
    /// Characters divided by four — a rough token estimate.
    ///
    /// Real tokenisation differs per model and would mean pulling in a big
    /// dependency. We only ever use this to reject a model whose window is
    /// *clearly* too small, never to choose between two that both fit, so being
    /// approximately right is genuinely good enough.
    pub(crate) tokens: u64,
}

/// Walk the request body and work out what it needs.
///
/// The wrinkle: OpenAI's format allows `content` to be either a plain string,
/// or an array of parts when the message carries images. Both shapes are legal
/// and both arrive in practice, so we handle each.
pub(crate) fn inspect(body: &Value) -> Needs {
    let mut vision = false;
    let mut chars = 0usize;

    // `if let Some(x) = ...` means "if this optional value exists, name it x".
    // Rust has no null; a thing that might be missing is an `Option`, and the
    // compiler forces you to say what happens when it's absent. That is why
    // null-pointer crashes essentially don't happen in this language.
    if let Some(messages) = body["messages"].as_array() {
        for message in messages {
            // `match` is a switch that the compiler checks for completeness.
            match &message["content"] {
                // Simple case: content is just text.
                Value::String(text) => chars += text.len(),

                // Richer case: an array of parts, any of which may be an image.
                Value::Array(parts) => {
                    for part in parts {
                        match part["type"].as_str() {
                            // Three spellings because different clients emit
                            // different ones. Copilot, Cline and the OpenAI SDK
                            // do not agree.
                            Some("image_url") | Some("image") | Some("input_image") => {
                                vision = true
                            }
                            // Anything else, count its text toward the estimate.
                            _ => chars += part["text"].as_str().map(str::len).unwrap_or(0),
                        }
                    }
                }
                // Some other shape we don't recognise. Ignore rather than fail:
                // being wrong about the estimate is survivable, refusing a valid
                // request is not.
                _ => {}
            }
        }
    }

    Needs {
        vision,
        tools: body["tools"]
            .as_array()
            .map(|t| !t.is_empty())
            .unwrap_or(false),
        tokens: (chars / 4) as u64,
    }
}

/// Can this model serve this request at all?
///
/// The `known` flag carries something subtle. A generic OpenAI-compatible
/// provider returns a list of model ids and nothing else — no capabilities, no
/// context sizes. If we treated "no published capability" as "cannot do it",
/// every model from such a provider would be skipped forever and the provider
/// would be useless.
///
/// So: absence of evidence is permission. We only skip a model when the catalog
/// positively tells us it can't. That rule applies at two depths: a model with
/// no catalog entry at all (`known == false`), and a catalogued model whose
/// entry never stated capabilities (`caps_known == false`) — a generic
/// provider's row says `vision: false` meaning "unstated", and treating that
/// as "cannot" would skip every direct-provider model on every tools request.
/// Likewise `context == 0` means *unknown*, not *zero-sized*, and must not be
/// used to reject anything.
pub(crate) fn can_serve(model: &providers::CatalogModel, needs: &Needs, known: bool) -> bool {
    if !known {
        return true; // nothing published; let the provider decide
    }
    if model.caps_known {
        if needs.vision && !model.vision {
            return false;
        }
        if needs.tools && !model.tools {
            return false;
        }
    }
    if model.context > 0 && needs.tokens > model.context {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_model_is_never_skipped() {
        // A generic provider publishes ids and nothing else. Treating silence as
        // "cannot" would make every such provider useless.
        let blank = providers::CatalogModel::default();
        let needs = Needs {
            vision: true,
            tools: true,
            tokens: 999_999,
        };
        assert!(can_serve(&blank, &needs, false));
    }

    #[test]
    fn a_known_model_is_skipped_on_what_it_lacks() {
        // `caps_known` is what makes the false a fact rather than a default.
        let text_only = providers::CatalogModel {
            context: 8192,
            caps_known: true,
            ..Default::default()
        };
        let needs = Needs {
            vision: true,
            tools: false,
            tokens: 10,
        };
        assert!(!can_serve(&text_only, &needs, true));
    }

    #[test]
    fn unstated_capabilities_never_skip() {
        // A generic provider's catalog entry says `vision: false, tools: false`
        // because it said NOTHING — the fields defaulted. Treating that as
        // "cannot" would skip every direct-provider model on every tools
        // request, silently and forever. The entry is in the catalog (known),
        // but its capabilities are not, so the request goes through.
        let unstated = providers::CatalogModel {
            context: 8192,
            ..Default::default()
        };
        let needs = Needs {
            vision: true,
            tools: true,
            tokens: 10,
        };
        assert!(can_serve(&unstated, &needs, true));

        // Context is a separate fact: when published it still applies, whatever
        // the capability fields do or don't say.
        let too_long = Needs {
            vision: false,
            tools: false,
            tokens: 9_000,
        };
        assert!(!can_serve(&unstated, &too_long, true));
    }

    #[test]
    fn an_unknown_context_never_rejects() {
        // `context: 0` means unpublished, not zero-sized.
        let unsized_model = providers::CatalogModel {
            tools: true,
            caps_known: true,
            ..Default::default()
        };
        let needs = Needs {
            vision: false,
            tools: true,
            tokens: 500_000,
        };
        assert!(can_serve(&unsized_model, &needs, true));
    }
}

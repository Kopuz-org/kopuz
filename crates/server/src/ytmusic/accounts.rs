//! The channels a Google account holds: its own, plus any brand accounts.
//! Each has its own YouTube Music library; a brand account is acted as by
//! sending its `pageId` (see [`super::innertube::signed`]).

use serde_json::{Value, json};

use super::clients::WEB_REMIX;
use super::innertube;

/// One channel the signed-in Google account can act as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtAccount {
    /// `None` for the Google account itself, else the brand account's page id.
    pub page_id: Option<String>,
    pub name: String,
    pub handle: Option<String>,
}

/// List the channels under the session in `cookies`. Asked as the Google
/// account itself, so a stale brand account choice cannot hide the list
/// that would fix it.
pub async fn list(cookies: &str) -> Result<Vec<YtAccount>, String> {
    let json = innertube::as_account(
        None,
        innertube::post(WEB_REMIX, "account/accounts_list", json!({}), Some(cookies)),
    )
    .await?;
    parse(&json)
}

/// Every `accountItem` in the switcher, wherever YouTube nests it. Items
/// without a page id other than the selected one are other Google sessions
/// in the same cookie jar, which a page id cannot reach, so they are dropped.
pub fn parse(json: &Value) -> Result<Vec<YtAccount>, String> {
    let mut items = Vec::new();
    collect(&json["actions"], &mut items);
    if items.is_empty() {
        return Err("accounts_list returned no accountItem".to_string());
    }
    let parsed: Vec<(bool, YtAccount)> = items.into_iter().filter_map(account).collect();
    let own = parsed
        .iter()
        .position(|(selected, a)| *selected && a.page_id.is_none())
        .or_else(|| parsed.iter().position(|(_, a)| a.page_id.is_none()));
    Ok(parsed
        .into_iter()
        .enumerate()
        .filter(|(i, (_, a))| a.page_id.is_some() || Some(*i) == own)
        .map(|(_, (_, a))| a)
        .collect())
}

fn collect<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "accountItem" {
                    out.push(child);
                } else {
                    collect(child, out);
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|child| collect(child, out)),
        _ => {}
    }
}

fn account(item: &Value) -> Option<(bool, YtAccount)> {
    let page_id = item["serviceEndpoint"]["selectActiveIdentityEndpoint"]["supportedTokens"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|token| token["pageIdToken"]["pageId"].as_str())
        .map(str::to_owned);
    let handle = text(&item["channelHandle"])
        .or_else(|| text(&item["accountByline"]).filter(|byline| byline.starts_with('@')));
    Some((
        item["isSelected"].as_bool().unwrap_or(false),
        YtAccount {
            page_id,
            name: text(&item["accountName"])?,
            handle,
        },
    ))
}

fn text(v: &Value) -> Option<String> {
    let joined = match v["simpleText"].as_str() {
        Some(simple) => simple.to_string(),
        None => v["runs"]
            .as_array()?
            .iter()
            .filter_map(|run| run["text"].as_str())
            .collect(),
    };
    let joined = joined.trim().to_string();
    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/accounts_list.json");

    #[test]
    fn lists_the_google_account_and_its_brand_accounts() {
        let json: Value = serde_json::from_str(FIXTURE).unwrap();
        let accounts = parse(&json).unwrap();
        assert_eq!(
            accounts,
            vec![
                YtAccount {
                    page_id: None,
                    name: "Ada Example".into(),
                    handle: Some("@adaexample".into()),
                },
                YtAccount {
                    page_id: Some("112233445566778899000".into()),
                    name: "Ada Records".into(),
                    handle: Some("@adarecords".into()),
                },
            ]
        );
    }

    #[test]
    fn another_google_session_in_the_jar_is_not_a_channel() {
        let item = |name: &str, selected: bool, page: Option<&str>| {
            let mut tokens = vec![json!({ "accountStateToken": {} })];
            if let Some(page) = page {
                tokens.push(json!({ "pageIdToken": { "pageId": page } }));
            }
            json!({ "accountItem": {
                "accountName": { "simpleText": name },
                "isSelected": selected,
                "serviceEndpoint": { "selectActiveIdentityEndpoint": { "supportedTokens": tokens } },
            }})
        };
        let json = json!({ "actions": [[
            item("Other login", false, None),
            item("Me", true, None),
            item("Brand", false, Some("42")),
        ]]});
        let names: Vec<String> = parse(&json).unwrap().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Me", "Brand"]);
    }

    #[test]
    fn a_response_without_the_switcher_is_an_error() {
        assert!(parse(&json!({ "actions": [] })).is_err());
    }
}

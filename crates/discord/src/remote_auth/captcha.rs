// https://docs.discord.food/topics/captcha-handling
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::Value;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub(super) const TRIES: usize = 3;
pub(super) const WAIT: Duration = Duration::from_secs(10 * 60);
const MAX_REQUEST: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Captcha {
    sitekey: String,
    rqdata: Option<String>,
    rqtoken: Option<String>,
    session_id: Option<String>,
}

impl Captcha {
    pub(super) fn from_body(body: &Value) -> Option<Self> {
        if !body.get("captcha_key").is_some_and(Value::is_array)
            || body.get("captcha_service").and_then(Value::as_str) != Some("hcaptcha")
        {
            return None;
        }
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            sitekey: text("captcha_sitekey")?,
            rqdata: text("captcha_rqdata"),
            rqtoken: text("captcha_rqtoken"),
            session_id: text("captcha_session_id"),
        })
    }

    pub(super) fn headers<'a>(&'a self, solution: &'a str) -> Vec<(&'static str, &'a str)> {
        let mut headers = vec![("x-captcha-key", solution)];
        if let Some(rqtoken) = &self.rqtoken {
            headers.push(("x-captcha-rqtoken", rqtoken));
        }
        if let Some(session_id) = &self.session_id {
            headers.push(("x-captcha-session-id", session_id));
        }
        headers
    }
}

pub(super) struct Page {
    pub(super) url: String,
    path: String,
    html: String,
    listener: TcpListener,
}

impl Page {
    pub(super) async fn serve(captcha: &Captcha) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let path = format!("/captcha/{:032x}", rand::random::<u128>());
        Ok(Self {
            url: format!("http://127.0.0.1:{port}{path}"),
            html: page(captcha),
            path,
            listener,
        })
    }

    /// Browsers open spare connections they may never use, so requests are served side by side.
    pub(super) async fn solution(&self) -> String {
        let mut pending = FuturesUnordered::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    if let Ok((stream, _)) = accepted
                        && pending.len() < MAX_CONNECTIONS
                    {
                        pending.push(tokio::time::timeout(REQUEST_TIMEOUT, self.answer(stream)));
                    }
                }
                Some(done) = pending.next(), if !pending.is_empty() => match done {
                    Ok(Ok(Some(solution))) => return solution,
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => log::debug!("Captcha page request failed: {error}"),
                    Err(_) => log::debug!("Captcha page request timed out"),
                },
            }
        }
    }

    async fn answer(&self, mut stream: TcpStream) -> std::io::Result<Option<String>> {
        let Some((head, body)) = read_request(&mut stream).await? else {
            return Ok(None);
        };
        let mut words = head.split(' ');
        let (method, path) = (words.next(), words.next());
        if path != Some(self.path.as_str()) {
            respond(&mut stream, "404 Not Found", "text/plain", "").await?;
            return Ok(None);
        }
        match method {
            Some("GET") => {
                respond(
                    &mut stream,
                    "200 OK",
                    "text/html; charset=utf-8",
                    &self.html,
                )
                .await?;
                Ok(None)
            }
            Some("POST") if !body.trim().is_empty() => {
                respond(&mut stream, "204 No Content", "text/plain", "").await?;
                Ok(Some(body.trim().to_owned()))
            }
            _ => {
                respond(&mut stream, "400 Bad Request", "text/plain", "").await?;
                Ok(None)
            }
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<(String, String)>> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || request.len() + read > MAX_REQUEST {
            return Ok(None);
        }
        request.extend_from_slice(&chunk[..read]);
        let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&request[..end]).into_owned();
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        if length > MAX_REQUEST {
            return Ok(None);
        }
        if request.len() >= end + 4 + length {
            let body = String::from_utf8_lossy(&request[end + 4..end + 4 + length]).into_owned();
            return Ok(Some((head, body)));
        }
    }
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

fn page(captcha: &Captcha) -> String {
    // JSON strings can still close the script element, so `</` is escaped too.
    let script = |value: &str| {
        serde_json::to_string(value)
            .unwrap_or_default()
            .replace("</", "<\\/")
    };
    let sitekey = script(&captcha.sitekey);
    let rqdata = captcha.rqdata.as_deref().map_or("null".to_owned(), script);
    format!(
        r#"<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">
<title>Log in to zcord</title>
<style>
  body {{ margin: 0; min-height: 100vh; display: flex; flex-direction: column; align-items: center;
    justify-content: center; gap: 16px; background: #1a1a1e; color: #dcdcdf;
    font: 15px system-ui, sans-serif; text-align: center; }}
  h1 {{ margin: 0; font-size: 20px; }}
  p {{ margin: 0; color: #96979e; }}
</style>
<h1>Log in to zcord</h1>
<p id="note">Discord wants to check you're human before it finishes your QR login.</p>
<div id="captcha"></div>
<script>
  const sitekey = {sitekey};
  const rqdata = {rqdata};
  const note = document.getElementById("note");
  function ready() {{
    const id = hcaptcha.render("captcha", {{ sitekey, theme: "dark", callback: solved }});
    if (rqdata) hcaptcha.setData(id, {{ rqdata }});
  }}
  async function solved(solution) {{
    note.textContent = "Sending it to zcord...";
    try {{
      const answer = await fetch(location.pathname, {{ method: "POST", body: solution }});
      note.textContent = answer.ok
        ? "Done. You can close this page and go back to zcord."
        : "zcord didn't take it. Start the QR login again in zcord.";
    }} catch {{
      note.textContent = "zcord stopped waiting. Start the QR login again in zcord.";
    }}
  }}
</script>
<script src="https://js.hcaptcha.com/1/api.js?render=explicit&onload=ready&recaptchacompat=off&host=discord.com" async defer></script>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn captcha() -> Captcha {
        Captcha::from_body(&json!({
            "captcha_key": ["captcha-required"],
            "captcha_service": "hcaptcha",
            "captcha_sitekey": "fixture-sitekey",
            "captcha_rqdata": "data</script>",
            "captcha_rqtoken": "fixture-rqtoken",
            "captcha_session_id": "fixture-session",
        }))
        .unwrap()
    }

    #[test]
    fn reads_hcaptcha_requests_and_builds_retry_headers() {
        let captcha = captcha();
        assert_eq!(captcha.rqdata.as_deref(), Some("data</script>"));
        assert_eq!(
            captcha.headers("solved"),
            [
                ("x-captcha-key", "solved"),
                ("x-captcha-rqtoken", "fixture-rqtoken"),
                ("x-captcha-session-id", "fixture-session"),
            ]
        );
        assert_eq!(
            Captcha::from_body(&json!({
                "captcha_key": ["captcha-required"],
                "captcha_service": "recaptcha",
                "captcha_sitekey": "key",
            })),
            None
        );
        assert_eq!(Captcha::from_body(&json!({"message": "no"})), None);
    }

    #[tokio::test]
    async fn serves_the_page_only_at_its_path_and_takes_the_solution() {
        let page = Page::serve(&captcha()).await.unwrap();
        let url = page.url.clone();
        let solution = tokio::spawn(async move { page.solution().await });
        let client = reqwest::Client::new();
        let html = client.get(&url).send().await.unwrap().text().await.unwrap();
        assert!(html.contains("host=discord.com"), "{html}");
        assert!(html.contains(r#""fixture-sitekey""#), "{html}");
        assert!(html.contains(r#""data<\/script>""#), "{html}");
        let elsewhere = format!("{}/guess", url.rsplit_once('/').unwrap().0);
        assert_eq!(
            client.get(elsewhere).send().await.unwrap().status(),
            reqwest::StatusCode::NOT_FOUND
        );
        client.post(&url).body("solved").send().await.unwrap();
        assert_eq!(solution.await.unwrap(), "solved");
    }
}

use anyhow::Result;
use assert2::check;
use http::StatusCode;
use serde::Deserialize;
use std::time::Duration;
use std::time::Instant;

// -- Server --------------------------------------------------------------
mod server {
    use anyhow::Result;
    use server_axum::app;
    use std::net::{SocketAddr, TcpListener};
    use url::Url;

    pub fn launch_server_axum() -> Result<Url> {
        let listener = TcpListener::bind("0.0.0.0:0".parse::<SocketAddr>()?)?;
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app().into_make_service())
                .await
                .unwrap();
        });
        let url = Url::parse(&format!("http://{}:{}", addr.ip(), addr.port()))?;
        Ok(url)
    }
}

// -- Client / User-agent -------------------------------------------------
mod client {
    use anyhow::{anyhow, Result};
    use reqwest::{Client, Method, Request, Response, StatusCode};
    use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Error, Middleware, Next};
    use std::time::{Duration, Instant};
    use task_local_extensions::Extensions;

    struct RetryAfterMiddleware;

    #[async_trait::async_trait]
    impl Middleware for RetryAfterMiddleware {
        async fn handle(
            &self,
            req: Request,
            extensions: &mut Extensions,
            next: Next<'_>,
        ) -> reqwest_middleware::Result<Response> {
            // Cloning the request object before-the-fact is not ideal..
            // However, if the body of the request is not static, e.g of type `Bytes`,
            // the Clone operation should be of constant complexity and not O(N)
            // since the byte abstraction is a shared pointer over a buffer.
            let cloned_request = req.try_clone().ok_or_else(|| {
                Error::Middleware(anyhow!(
                    "Request object is not clonable. Are you passing a streaming body?".to_string()
                ))
            })?;
            let cloned_next = next.clone();
            let mut check_retry = true;
            let mut res = next.run(req, extensions).await;
            let start_at = Instant::now();
            while check_retry {
                println!(
                    "{:?} : check info, then continue, retry or follow",
                    start_at.elapsed()
                );
                check_retry = false;
                if let Ok(ref response) = res {
                    if response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status() == StatusCode::SERVICE_UNAVAILABLE
                        || response.status() == StatusCode::MOVED_PERMANENTLY
                        || response.status() == StatusCode::FOUND
                        || response.status() == StatusCode::SEE_OTHER
                        || response.status() == StatusCode::TEMPORARY_REDIRECT
                    {
                        if let Some(retry_after) = response.headers().get(http::header::RETRY_AFTER)
                        {
                            match retry_after.to_str().unwrap().parse::<u64>() {
                                Ok(secs) => {
                                    tokio::time::sleep(Duration::from_secs(secs)).await;
                                    check_retry = true;
                                    let next = cloned_next.clone();
                                    let request = if response.status() == StatusCode::SEE_OTHER {
                                        //TODO change method and url
                                        let mut url = cloned_request.url().clone();
                                        if let Some(location) =
                                            response.headers().get(http::header::LOCATION)
                                        {
                                            url.set_path(location.to_str().unwrap());
                                        }
                                        let mut req = Request::new(Method::GET, url);
                                        *req.timeout_mut() = cloned_request.timeout().cloned();
                                        *req.headers_mut() = cloned_request.headers().clone();
                                        *req.version_mut() = cloned_request.version();
                                        req
                                    } else {
                                        cloned_request.try_clone().ok_or_else(|| {
                                            Error::Middleware(anyhow!(
                                                "Request object is not clonable. Are you passing a streaming body?".to_string()
                                            ))
                                        })?
                                    };
                                    res = next.run(request, extensions).await;
                                }
                                Err(_) => (), //TODO support other format
                            }
                        }
                    }
                }
            }
            res
        }
    }

    pub fn make_user_agent() -> Result<ClientWithMiddleware> {
        let custom = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 5 {
                attempt.error("too many redirects")
            } else if attempt.status() == StatusCode::SEE_OTHER {
                // headers are not available :-(, so prefer to stop
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let reqwest_client = Client::builder().redirect(custom).build()?;
        let client = ClientBuilder::new(reqwest_client)
            .with(RetryAfterMiddleware)
            .build();
        Ok(client)
    }
}

// -- Caller --------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct WorkOutput {
    nb_get_call: u16,
    duration: Duration,
}

#[tokio::test]
async fn polling_with_reqwest() -> Result<()> {
    let server_url = server::launch_server_axum()?;
    let user_agent = client::make_user_agent()?;

    let now = Instant::now();
    let response = user_agent
        .post(server_url.join("start_work")?)
        .send()
        .await?;

    check!(response.status() == StatusCode::OK);

    let body = response.json::<WorkOutput>().await?;
    dbg!(&body);
    check!(body.nb_get_call < 20);
    check!(body.duration < now.elapsed());
    check!(now.elapsed() < Duration::from_secs(21));

    Ok(())
}

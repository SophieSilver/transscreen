use std::{
    convert::Infallible,
    fmt::Debug,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration, borrow::Cow,
};

use futures::{Future, SinkExt};
use hyper::{
    service::{self, Service},
    Body, Method, Request, Response, Server, StatusCode,
};
use hyper_tungstenite::{HyperWebsocket, tungstenite::{Message, protocol::{CloseFrame, frame::coding::CloseCode}}};
use tower::{timeout::TimeoutLayer, Layer, ServiceBuilder};

#[derive(Debug, Clone, Copy)]
struct StaticState {
    index_html: &'static [u8],
    stylesheet: &'static [u8],
    script: &'static [u8],
}

struct InstantFuture<T>(Option<T>);

impl<T> Unpin for InstantFuture<T> {}

impl<T> Future for InstantFuture<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.0.take().expect("InstantFuture got polled twice"))
    }
}

#[derive(Debug, Clone)]
struct StaticPageService {
    state: StaticState,
}

impl Service<Request<Body>> for StaticPageService {
    type Response = Response<Body>;

    type Error = Infallible;

    type Future = InstantFuture<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let response = match (req.method(), req.uri().path()) {
            (&Method::GET, "/") => Response::new(self.state.index_html.into()),
            (&Method::GET, "/stylesheet") => Response::new(self.state.stylesheet.into()),
            (&Method::GET, "/script") => Response::new(self.state.script.into()),

            _ => Response::builder().status(404).body(Body::empty()).unwrap(),
        };

        InstantFuture(Some(Ok(response)))
    }
}

pub async fn run() {
    let state = StaticState {
        index_html: include_bytes!("../static/index.html"),
        stylesheet: &[],
        script: include_bytes!("../static/main.js"),
    };

    let svc = StaticPageService { state };
    let full_svc = ServiceBuilder::new()
        .layer(LogLayer)
        .layer(TimeoutLayer::new(Duration::from_secs(10)))
        .layer(WebSocketUpgradeLayer::new(handle_websocket))
        // .layer(LoadShedLayer::new())
        // .layer(BufferLayer::new(1))
        // .layer(RateLimitLayer::new(10, Duration::from_secs(30)))
        .service(svc);

    let make_svc = service::make_service_fn(|_conn| {
        let svc = full_svc.clone();

        async { Ok::<_, Infallible>(svc) }
    });

    let addr: SocketAddr = "0.0.0.0:9090".parse().unwrap();

    let server = Server::bind(&addr).serve(make_svc);
    _ = server.await;
}

#[derive(Debug, Clone, Copy)]
struct LogService<S> {
    inner: S,
}

impl<S, B> Service<Request<B>> for LogService<S>
where
    S: Service<Request<B>, Response = Response<B>> + Clone + Send + 'static,
    S::Error: Debug,
    S::Future: Send,
    B: Send + 'static,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        println!("REQUEST:  {} {}", req.method(), req.uri());
        let mut this = self.clone();

        Box::pin(async move {
            let resp = this.inner.call(req).await;
            match &resp {
                Ok(resp) => println!("RESPONSE: {:?}", resp.status()),
                Err(e) => println!("RESPONSE ERROR: {e:?}"),
            };

            resp
        })
    }
}

struct LogLayer;

impl<S> Layer<S> for LogLayer {
    type Service = LogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LogService { inner }
    }
}

#[derive(Debug)]
struct WebSocketUpgrade<S, F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut,
    Fut: Future<Output = ()>,
{
    inner: S,
    websocket_handler: F,
}

impl<S, F, Fut> WebSocketUpgrade<S, F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut,
    Fut: Future<Output = ()>,
{
    fn new(service: S, websocket_handler: F) -> Self {
        Self {
            inner: service,
            websocket_handler,
        }
    }
}

// implementing manually because derive macro gets confused when Fut isn't Clone
impl<S, F, Fut> Clone for WebSocketUpgrade<S, F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut + Clone,
    Fut: Future<Output = ()>, // this one doesn't have to be clone, it's returned by F
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            websocket_handler: self.websocket_handler.clone(),
        }
    }
}

// same story as with Clone
impl<S, F, Fut> Copy for WebSocketUpgrade<S, F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut + Copy,
    Fut: Future<Output = ()>, // this one doesn't have to be copy
    S: Copy,
{
}

impl<S, F, Fut, B> Service<Request<B>> for WebSocketUpgrade<S, F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    S: Service<Request<B>, Response = Response<B>> + Send,
    S::Future: Send + 'static,
    B: Default + Send + 'static,
    Self: Clone,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = Pin<Box<dyn Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // check request for websocket upgrade
        if !hyper_tungstenite::is_upgrade_request(&req) {
            return Box::pin(self.inner.call(req));
        }

        let mut this = self.clone();
        Box::pin(async move {
            let (response, websocket) = match hyper_tungstenite::upgrade(req, None) {
                Err(_) => {
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(B::default())
                        .unwrap())
                }
                Ok(pair) => pair,
            };

            let handler_fut = (this.websocket_handler)(websocket);
            tokio::spawn(handler_fut);
            
            // I want this Service to be a bit more flexible over the type of body, so instead of returning the
            // Response<Body> that hyper_tungstenite provides, I return a response with default body of the right type;
            let mut response_builder = Response::builder().status(response.status());
            *response_builder.headers_mut().unwrap() = response.headers().clone();
            let adapted_response = response_builder.body(B::default()).unwrap();
            
            Ok(adapted_response)
        })
    }
}

struct WebSocketUpgradeLayer<F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut,
    Fut: Future<Output = ()>,
{
    websocket_handler: F,
}

impl<F, Fut> WebSocketUpgradeLayer<F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut,
    Fut: Future<Output = ()>,
{
    fn new(f: F) -> Self {
        Self {
            websocket_handler: f,
        }
    }
}

impl<F, Fut, S> Layer<S> for WebSocketUpgradeLayer<F, Fut>
where
    F: FnMut(HyperWebsocket) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    type Service = WebSocketUpgrade<S, F, Fut>;

    fn layer(&self, inner: S) -> Self::Service {
        WebSocketUpgrade::new(inner, self.websocket_handler.clone())
    }
}

async fn handle_websocket(ws: HyperWebsocket) {
    println!("Got a websocket");
    let mut socket = ws.await.unwrap();
    socket.send(Message::Text("Hellooo from a websocket".to_string())).await.unwrap();
    socket.send(Message::Binary(vec![1, 2, 3, 4])).await.unwrap();
    socket.send(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: Cow::Borrowed("fuck you") }))).await.unwrap();
}

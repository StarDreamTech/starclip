use super::*;

// Custom loopback protocol, not HTTP: remote web pages cannot supply valid
// frames or the random connection credential. No contents are logged.
// Single worker and bounded frames/timeouts keep this a small local bridge. A
// hostile same-account process can still deny service; this is not a network API.
pub(super) fn start_ai_broker(
    context: &Arc<Context>,
    mut emit_pet: impl FnMut(&ai_access::PetSnapshot) -> Result<()> + Send + 'static,
) {
    use std::net::TcpListener;
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        return;
    };
    let Ok(address) = listener.local_addr() else {
        return;
    };
    // Blocking accept: the worker sleeps in the kernel until a client connects
    // instead of waking every 50 ms. It lives for the whole app process; the
    // weak context check below still stops serving once the context is gone.
    if let Ok(mut ai) = context.ai.lock() {
        ai.port = Some(address.port());
    } else {
        return;
    }
    let weak = Arc::downgrade(context);
    thread::spawn(move || {
        while weak.strong_count() > 0 {
            match listener.accept() {
                Ok((mut stream, address)) => {
                    if !address.ip().is_loopback() {
                        continue;
                    }
                    if weak.strong_count() == 0 {
                        break;
                    }
                    // Keep the stream explicitly blocking with bounded timeouts
                    // so fragmented frames wait for later packets.
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let request = ai_access::read_frame(
                        &mut std::io::BufReader::new(&mut stream),
                        ai_access::MAX_REQUEST,
                    )
                    .ok()
                    .flatten()
                    .and_then(|bytes| serde_json::from_slice::<ai_access::Request>(&bytes).ok());
                    let handled = (|| -> Result<()> {
                        let context = weak.upgrade().ok_or(ai_access::DENIED)?;
                        // The same lock order as mutations. Busy/frozen storage fails
                        // closed; guards live through socket write, so revoke/privacy
                        // changes cannot interleave between filtering and emission.
                        let storage = context.storage.try_lock().map_err(|_| ai_access::DENIED)?;
                        storage.writable().map_err(|_| ai_access::DENIED)?;
                        let store = lock(&context.store)?;
                        let mut ai = lock(&context.ai)?;
                        let response = ai.respond_with_pet(
                            &store,
                            request.ok_or(ai_access::DENIED)?,
                            &mut emit_pet,
                        );
                        ai_access::write_frame(&mut stream, &response)
                            .map_err(|_| ai_access::DENIED.to_string())
                    })();
                    if handled.is_err() {
                        let _ = ai_access::write_frame(
                            &mut stream,
                            &serde_json::json!({"error": ai_access::DENIED}),
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => thread::sleep(Duration::from_millis(200)),
            }
        }
    });
}

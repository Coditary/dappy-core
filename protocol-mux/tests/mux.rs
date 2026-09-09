use protocol_mux::{BackendSeq, ClientRole, ClientSeq, Id, MessageRemapper, Multiplexer};

#[test]
fn remapper_roundtrip() {
    let remapper = MessageRemapper::new();
    let client = ClientSeq::new(42);
    let backend = BackendSeq::new(7);
    remapper.map(client, backend);
    assert_eq!(remapper.lookup_client(backend), Some(client));
    assert_eq!(remapper.lookup_backend(client), Some(backend));
    assert_eq!(remapper.unmap(backend), Some(client));
}

#[test]
fn multiplexer_tracks_clients() {
    let mut mux = Multiplexer::<Id>::new();
    let editor = mux.attach_client(ClientRole::Editor);
    let agent = mux.attach_client(ClientRole::Agent);
    assert_eq!(mux.client_count(), 2);
    assert_eq!(mux.detach_client(editor), Some(ClientRole::Editor));
    assert_eq!(mux.detach_client(agent), Some(ClientRole::Agent));
    assert_eq!(mux.client_count(), 0);
}

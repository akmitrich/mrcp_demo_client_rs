# MRCP demo client for recognition in Rust
## UniMRCP client
This piece of software is based on the demo recog application https://github.com/unispeech/unimrcp/tree/master/platforms/unimrcp-client by **Arsen Chaloyan**.
To run it there must be a correct UniMRCP dir layout at `/opt/unimrcp`. Or somewhere else if you installed UniMRCP other way. You may also replace `unimrcp_client_create` with something else which takes your client and server configuration.

## Working example
No more. Is based on the crate https://github.com/akmitrich/uni_lib_client_rs. Just take a file from the only argument and send it to the MRCP server to recognize speech. All the parameters are hardcoded.

## Build
The dependency `uni_lib_client_rs` is expected to be in the next directory (look at the path in `Cargo.toml`). But you can rely on https://github.com/akmitrich/uni_lib_client_rs.
After this
```
cargo build
```
will take care of everything else)))

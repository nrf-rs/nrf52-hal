# CCM encryption demo

This CCM (cipher block chaining) encryption demo initialises a text message of the maximum size of 251 bytes and encrypts and decrypts it, measuring the time it takes. It then repeats the process with smaller and smaller chunks of data to demonstrate how long smaller packets take to process.

## How to run

Choose the microcontroller with one of the following features:
- 51
- 52810
- 52811
- 52832
- 52840

Also, if using `cargo-embed`, change the `chip` and `protocol` fields in [Embed.toml](Embed.toml).

This demo uses the [rtt-target](https://crates.io/crates/rtt-target) crate for communication.

If using `cargo-embed`, just run

```console
$ cargo embed --release --features=52832 --target=thumbv7em-none-eabihf
```

Replace `52832` and `thumbv7em-none-eabihf` with the correct feature and target for your microcontroller.

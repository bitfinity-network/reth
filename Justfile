# Lists all the available commands
default:
  @just --list


# Run the node as EVM archiver
[group('run')]
run_local url="https://block-extractor-testnet-1052151659755.europe-west9.run.app":
  cargo run -p reth -- node -vvv --http --http.port 8080 --http.addr 0.0.0.0 --http.api "debug,eth,net,trace,txpool,web3" --disable-discovery --ipcdisable --no-persist-peers -r {{url}} -i 10 -b 100 --max-fetch-blocks 5000 --log.file.directory ./target/logs --datadir ./target/reth


# Run all bitfinity tests
[group('test')]
bitfinity_test:
  cargo test bitfinity_test -p bitfinity-block-confirmation -p reth -p reth-downloaders -- --nocapture

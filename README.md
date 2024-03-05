# eth-indexer
Indexes Ethereum block rewards: MEV and tips.

## Run

* Requires an execution client running at `http://localhost:8545`.
* Will process `from_block` to `to_block`, see code.
* No state is stored.
* Don't use it in production, untested dirty script.

```
cargo run
```

## Test

```
cargo test
```
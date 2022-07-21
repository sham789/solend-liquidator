# Rust implementation of solend-liquidator



## Overview

This particular implementation spawns thousands of threads for concurrent processing thanks to `tokio`. Techincally, amount of threads per iteration equals:

```
threads_n = markets.len() * obligations.len()
```

## Usage

1. Install Solana CLI tools, Rust. 

2. Create liquidator keypair. Currently hardcoded to relative path `./solend-liquidator/private/liquidator_main.json`. Run in the root directory of the project.

```
solana-keygen new -o ./solend-liquidator/private/liquidator_main.json
```

2. To run test iteration. Run in the root directory of the project:

```
cd ./solend-liquidator
cargo test -- --nocapture
```

3. To run actual eternal liquidator.

```
cd ./solend-liquidator
cargo run --release
```
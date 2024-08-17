```bash
# build the project
cargo build --release

# run the daemon
sudo ./target/release/ore-cli-wrapper \
    --cores <num_cores> \
    --keypair <path_to_keypair> \
    --fee-payer <path_to_fee_payer_keypair> \
    --dynamic-fee \
    --dynamic-fee-url <rpc_dynamic_fee_url> \
    --rpc <rpc_url>

# check the daemon status
sudo cat /tmp/ore_miner/daemon.log
sudo cat /tmp/ore_miner/daemon.err

# check logs
sudo ls /tmp/ore_miner/logs

# stop the daemon
sudo kill $(sudo cat /tmp/ore_miner/process.pid)
```

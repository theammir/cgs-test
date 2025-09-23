Normally, there's a fancy description. Instead,

# Setup

1. Make sure to have Solana CLI installed:

```bash
$ sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
```

2. Make sure to have Anchor CLI installed:

```bash
$ yarn
or similar
```

3. Make sure to have a valid Solana keypair at `~/.config/solana/id.json`, or edit `provider.wallet` in `Anchor.toml`:

```bash
$ solana-keygen new -o ~/.config/solana/id.json
```

4. Rename `env.example` to `.env` and tweak the variables for localnet
   (`SAS_RPC_URL=http://127.0.0.1:8899`) or devnet
   (`SAS_RPC_URL=https://api.devnet.solana.com`).

# Usage

### Backend

To run the backend, simply do

```bash
$ cargo run
```

It is listening to `POST` requests at `http://localhost:3000`.

##### POST `/verification`

Example body:

```json
{
  "address": "5HnSzDfPiTEb7oxPwAfGrBoExqYb2hoXtwDjN97sXu9h"
}
```

Example response:

```json
{
  "age": true,
  "country": true
}
```

> [!NOTE]
> *If the address is an invalid pubkey, the response will be falsy.*

##### GET `/validate`

Example query:

```
?address=5HnSzDfPiTEb7oxPwAfGrBoExqYb2hoXtwDjN97sXu9h
```

Example response:

```json
{"address":"5HnSzDfPiTEb7oxPwAfGrBoExqYb2hoXtwDjN97sXu9h","valid":true}
```

### On-chain validator localnet testing

This one's tricky on my machine.

1. Build the program:

```bash
$ anchor build
```

2. Run a local validator in a separate terminal, and then run the tests skipping pre-deployment:

```bash
$ anchor localnet &
$ anchor test --skip-local-validator --skip-deploy
```

> [!NOTE]
> You could simply try `anchor test`, but it didn't reliably preload the SAS program at genesis for me. I should really make a Dockerfile for this.

### On-chain validator deployment

```bash
$ anchor localnet &
$ anchor deploy
```

or

```bash
$ anchor deploy --provider.cluster devnet
```

# CMS Attestation Key Pair Generation

This document describes the one-off procedure to generate and manage the cryptographic key pair used for CMS-signed server attestations. The CMS (Tessera Synth website) signs attestations that the `directory` service verifies to authenticate official server advertisements.

## Key Pair Format

- **Private key:** Ed25519, PKCS#8 PEM format
  - Consumed by: CMS (Tessera website/cms-admin)
  - Environment variable: `TESSERA_ATTESTATION_SIGNING_KEY`
  - Status: **Secret** — never committed, stored in password manager only
  
- **Public key:** Ed25519, SPKI PEM format
  - Consumed by: `directory` service (validates signatures)
  - Environment variable: `TESSERA_ATTESTATION_PUBLIC_KEY`
  - Status: **Non-secret** — safe to commit in config, visible to anyone

## Generating the Key Pair

**Important:** This procedure is run once, locally, not in CI. The private key is immediately secured in a password manager and never pushed to any repository.

### Step 1: Generate Private Key

```bash
openssl genpkey -algorithm ed25519 -out /tmp/tessera-attestation-private.pem
```

This creates a PKCS#8 PEM private key at `/tmp/tessera-attestation-private.pem`.

### Step 2: Extract Public Key

```bash
openssl pkey -in /tmp/tessera-attestation-private.pem -pubout -out /tmp/tessera-attestation-public.pem
```

This extracts the SPKI PEM public key from the private key.

**Verify both files were created:**

```bash
ls -l /tmp/tessera-attestation-{private,public}.pem
```

**Inspect the public key (safe to view):**

```bash
cat /tmp/tessera-attestation-public.pem
```

## Deployment: Securing the Keys

### Private Key → CMS Secret

1. **Open your password manager** (1Password, Bitwarden, or similar)
2. **Create a new secure note** containing the entire private key file content (do NOT modify it)
3. **Label it:** `tessera-cms TESSERA_ATTESTATION_SIGNING_KEY`
4. **Store the PEM block exactly as-is** — multiline format, no modifications, no escaped newlines

When deploying the CMS to Dokploy:
- Create a **Dokploy secret** named `TESSERA_ATTESTATION_SIGNING_KEY`
- Paste the PEM block from password manager into the Dokploy UI or environment configuration
- The code handles both:
  - Multiline PEM (recommended: copy-paste as-is)
  - Single-line with literal `\n` escape sequences (alternative, less common)

### Public Key → Directory Config

1. **View the public key (always safe to share):**
   ```bash
   cat /tmp/tessera-attestation-public.pem
   ```

2. **Add it to `.env.example` and `.env`** in the `tessera-core/server/` directory:
   ```bash
   # tessera-core/server/.env.example
   # Ed25519 public key (SPKI PEM) for verifying CMS-signed server attestations.
   # Generated once via: openssl pkey -in /tmp/tessera-attestation-private.pem -pubout
   # Value: copy the entire output from `cat /tmp/tessera-attestation-public.pem`
   TESSERA_ATTESTATION_PUBLIC_KEY="<entire-public-key-including-begin-end-markers>"
   ```

3. **Load it in the Directory service** at startup (via environment variables, config file, or secrets manager)

## Attestation Format and Usage

When a server requests an attestation from the CMS, the CMS signs the following JWT payload using EdDSA (Ed25519 algorithm):

```json
{
  "iss": "tessera-cms",
  "sub": "<server-slug>",
  "iat": <unix-timestamp>,
  "exp": <unix-timestamp>
}
```

The `directory` service verifies the JWT signature using the public key. A valid signature proves:
- The CMS authorized this server
- The server's slug matches the JWT's `sub` claim
- The attestation has not expired (current time is between `iat` and `exp`)

## Key Rotation Procedure

If you need to rotate the keys (security best practice is annually, or on suspected compromise):

1. **Generate a new pair** using the procedure above (Step 1 & 2)
2. **Update both deployed secrets:**
   - `TESSERA_ATTESTATION_SIGNING_KEY` (new private key in CMS Dokploy secret)
   - `TESSERA_ATTESTATION_PUBLIC_KEY` (new public key in Directory environment/config)
3. **Redeploy both services** (CMS and Directory)
4. **Trigger re-attestation** for all active servers
   - Servers holding old attestations will be rejected until they request new ones
   - This is intentional security: forces regular contact with the CMS

**Important:** Delete the old private key from your password manager after confirming the new one works.

## Security Considerations

- **Never commit any key** (public or private) to any Git repository. The repository's secrets detection hook blocks Ed25519 private key patterns.
- **Never log or print keys** to console output, log files, or debug traces.
- **Private key = full authority.** If the CMS Dokploy secret is compromised, an attacker can forge attestations for any server. Rotate immediately if suspected.
- **Public key rotation is player-transparent.** Players automatically receive new attestations on their next server join; no client update needed.
- **No hard expiration.** Rotate on your schedule (yearly recommended) or on incidents.

## Testing and Verification

After key generation, the `tessera-core/directory/tests/` module includes tests that:
- Verify the public key can validate signatures
- Check JWT structure and claims
- Validate expiration checks

Run the directory tests locally:

```bash
cd tessera-core
cargo test -p directory
```

To manually test the signing/verification flow:
1. Implement a small test utility that reads both keys from disk
2. Sign a test message with the private key
3. Verify the signature with the public key
4. This confirms the pair is valid before deployment

## Implementation Integration

**CMS (tessera-administration/website/cms/):**
- Reads `TESSERA_ATTESTATION_SIGNING_KEY` environment variable at startup
- On server attestation endpoint, creates a JWT with the payload above
- Signs using Ed25519 (EdDSA)
- Returns signed JWT to the requesting server

**Directory (tessera-core/directory/):**
- Reads `TESSERA_ATTESTATION_PUBLIC_KEY` environment variable at startup
- Receives attestation JWTs during server join requests
- Verifies signature using Ed25519
- Checks JWT expiration (current Unix time must be within `iat` and `exp`)
- Accepts or rejects based on verification result

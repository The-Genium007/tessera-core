# Fixtures — test du fil (`verifies_real_cms_wire_token`)

`wire-token.jwt` et `wire-public.pem` sont produits par le **vrai signeur CMS**
(`tessera-administration/website/cms/src/lib/attestationSigner.ts`, via `jose`), pas par un
ré-encodage Rust — c'est le point du « test du fil » (CLAUDE.md : tester le fil, pas chaque côté
isolément).

- `wire-token.jwt` : un JWT EdDSA valide, `iss="tessera-cms"`, `sub="wire-fixture-slug"`,
  `exp` ≈ l'an 2100 (pas une bombe à retardement). C'est un token public, pas un secret.
- `wire-public.pem` : la clé **publique** SPKI correspondante. La clé privée utilisée pour signer
  n'est **jamais** écrite sur disque ni committée — elle n'existe que dans la mémoire du process
  Node le temps de la génération.

## Régénérer

Si le contrat JWT change (issuer, claims, algorithme), régénérer depuis
`tessera-administration/website/cms` :

```bash
node --input-type=module -e '
import { generateKeyPair, exportPKCS8, exportSPKI } from "jose";
import { writeFileSync } from "node:fs";
const { publicKey, privateKey } = await generateKeyPair("EdDSA", { extractable: true });
process.env.TESSERA_ATTESTATION_SIGNING_KEY = await exportPKCS8(privateKey);
const { signOfficialAttestation } = await import("./src/lib/attestationSigner.ts");
const ttl = 4102444800 - Math.floor(Date.now()/1000); // exp ≈ 2100
const { attestation } = await signOfficialAttestation("wire-fixture-slug", ttl);
const dir = "../../../tessera-core/directory/tests/fixtures";
writeFileSync(dir + "/wire-token.jwt", attestation);      // token (public, pas un secret)
writeFileSync(dir + "/wire-public.pem", await exportSPKI(publicKey)); // clé PUBLIQUE seule
console.log("fixtures écrits");
'
```

`node_modules` du CMS doit être installé au préalable. Si l'exécution TS directe échoue (selon la
version de Node), builder le module d'abord ou pointer le `.js` compilé — l'important est que le
token provienne du **vrai** signeur, jamais d'un ré-encodage Rust.

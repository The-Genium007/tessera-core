# ADR 0006: Distribution des modsets & signature des manifestes

- **Statut :** accepté
- **Date :** 2026-06-26

> Note : les numéros 0004 (toolchain Windows) et 0005 (port-vs-rebuild Cyberverse)
> sont réservés par le plan de phase 0-B et seront créés à son exécution.

## Contexte

Le launcher TesseraSynth (Tauri, développé par Codex) doit installer et mettre à jour
les mods sur l'installation Cyberpunk 2077 du joueur. Le modèle retenu est **manifest-first** :
le launcher télécharge un manifeste distant, le vérifie, compare à l'état local, télécharge
les paquets manquants/modifiés, vérifie leur intégrité, et applique en transactionnel.

Le launcher est le **consommateur**. Il manquait le **producteur** : l'endroit où les
manifestes et artefacts sont publiés, et l'outil qui les fabrique. Une revue de sécurité a
par ailleurs montré que sans authentification du manifeste, un manifeste forgé conduit à
exécuter du code non vérifié (RCE). La signature n'est donc pas optionnelle.

Contraintes : self-host/per-server, pas de serveur custom à maintenir pour la distribution
de base, URLs stables « définitives », aucune redistribution d'asset CDPR.

## Décision

**1. Hébergement 100 % statique, sans backend ni authentification.**
- Manifestes servis par **GitHub Pages** depuis le dépôt `The-Genium007/distribution`.
- Artefacts (`.zip`) en **GitHub Releases** du même dépôt (immuables, versionnés par tag).

**2. URLs canoniques (figées, ne changent plus) :**
- Manifeste : `https://the-genium007.github.io/distribution/modsets/<canal>/latest.json`
- Signature : `https://the-genium007.github.io/distribution/modsets/<canal>/latest.json.sig`
- Asset : `https://github.com/The-Genium007/distribution/releases/download/modset-v<version>/<id>.zip`
- Canaux : `stable` (défaut), `playtest`, `dev`.

**3. Signature Ed25519 détachée.**
- Le `.sig` est le **base64 d'une signature Ed25519 de 64 octets** calculée sur les
  **octets bruts exacts** de `latest.json` (aucune canonicalisation JSON).
- L'outil sérialise le manifeste **une seule fois** en octets, signe ces octets, écrit ces
  **mêmes** octets dans `latest.json`, et `base64(sig)` dans `latest.json.sig` → identité
  octet-pour-octet garantie entre ce qui est signé et ce qui est publié.
- Le champ JSON `signature` interne est **déprécié** (ignoré à la vérification).
- Clé publique de production (épinglée en dur dans le launcher) :
  `cRjBE8ZUs4WvGVX8sUC5r2eLN0nHtBTuVyUsFQSLJqM=`.
- Clé privée : **hors dépôt** (gestionnaire de mots de passe / secret CI), lue par l'outil
  via la variable d'env `TESSERA_SIGNING_KEY` (seed base64 32 octets). Rotation = nouvelle
  paire + ré-épinglage côté launcher.

**4. Intégrité des paquets.** Chaque package porte un `sha256` réel + `size`. Aucun
placeholder n'est jamais émis. Le launcher refuse tout hash vide/placeholder (fatal).

**5. Sécurité de transport.** URLs `https://` uniquement, allowlist d'hôtes
(`github.com`, `objects.githubusercontent.com`, `the-genium007.github.io`). Extraction
zip-slip-safe côté launcher (chemins relatifs uniquement, pas de `..`/absolu/symlink).

**6. Outillage producteur : crate `tools/release` (`tessera-release`).**
Une commande lit un descripteur `release.toml`, construit les zips (overlay enraciné jeu,
chemins relatifs), calcule `sha256`+`size`, émet `latest.json` + `latest.json.sig` signés,
puis publie (Release + push du dossier `distribution/` mirroré vers le dépôt public).

**7. Layout monorepo.** Le contenu publié vit dans `distribution/` (mirroré vers
`The-Genium007/distribution` par subtree, comme `web/`). L'outil `tessera-release` vit dans
`tools/release/` (non mirroré).

## Conséquences

**Positives.**
- Zéro infra à opérer : Pages + Releases suffisent, gratuit, public, durable.
- Signer les octets bruts élimine toute ambiguïté de canonicalisation cross-langage.
- Découplage producteur/consommateur : « pousser un mod » = publier un nouveau manifeste,
  sans jamais retoucher le launcher.
- Le pipeline est testable et validable de bout en bout sur macOS avec un zip stub, sans jeu.

**Négatives / risques.**
- Cache CDN de GitHub Pages (~minutes) : un nouveau `latest.json` peut être servi avec un
  léger délai (cohérence éventuelle, acceptable).
- La clé privée est un secret critique : sa fuite impose une rotation + mise à jour launcher.
- Le layout overlay concret de chaque dépendance (RED4ext, CET, redscript, Codeware) dépend
  de la validation toolchain 2.31 (phase 0-B) ; les zips réels sont donc bloqués sur 0-B.

## Alternatives considérées

- **Org GitHub `TesseraSynth` dédiée** (URLs déjà hardcodées par Codex) : écartée au profit
  de `The-Genium007/distribution` (compte existant) ; Codex a mis à jour ses URLs en
  conséquence.
- **Signature inline dans le JSON** (champ `signature`) : écartée — re-sérialiser un JSON à
  l'identique octet-pour-octet entre deux langages est fragile ; le `.sig` détaché est sûr.
- **Lier les dépendances tierces vers leur upstream** au lieu de les héberger : écartée —
  fragile (liens morts), non reproductible. On héberge des copies épinglées (licences MIT
  des outils le permettent ; LICENSE jointes).
- **Sigstore/cosign (keyless OIDC)** : écartée pour l'instant — plus lourd à embarquer dans
  le launcher Tauri qu'un simple Ed25519 épinglé.
- **Serveur de distribution custom** : écarté — contredit le modèle self-host léger et
  ajoute une surface à opérer/sécuriser.

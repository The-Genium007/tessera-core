## Résumé

## Checklist

- [ ] `cargo test` passe (et `cargo test --features gns` si la signature de
      `gateway_main` ou les types de `handoff.rs` sont touchés)
- [ ] `cargo fmt && cargo clippy` sans warning sur le code modifié
- [ ] TDD respecté (test rouge → vert) côté Rust ; vérification manuelle documentée
      ci-dessous côté `client-mod/`
- [ ] Aucun asset CD Projekt Red ajouté (voir [CONTRIBUTING.md](../CONTRIBUTING.md))
- [ ] ADR ajouté/amendé si cette PR change une décision d'architecture

## Vérification manuelle (client-mod uniquement)

## Notes de revue

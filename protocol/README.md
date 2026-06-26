# protocol/ — Contrat réseau partagé (FlatBuffers)

Source de vérité des messages client↔serveur. Les schémas `.fbs` vivent dans
`schema/`. Génération :

- Rust (serveur) : `flatc --rust -o ../server/src/generated schema/<x>.fbs`
- C++ (client, Windows) : `flatc --cpp -o ../client-mod/generated schema/<x>.fbs`

Phase 0-A : dossier vide (le premier schéma arrive en Phase 0-C). Installer le
compilateur : `brew install flatbuffers` (macOS).

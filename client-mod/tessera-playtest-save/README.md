# tessera-playtest-save — lancement direct sur une sauvegarde dédiée

Fait partie de la chaîne "clic Play → dans le monde, sans rien voir" (spec
`docs/superpowers/specs/2026-07-05-playtest-shards-design.md` §#3) : avec `-skipStartScreen` (déjà
géré côté launch args), ce module couvre le morceau restant — sauter le menu principal et charger
directement une sauvegarde connue, sans passer par Continuer/Nouvelle partie.

**Plateforme :** Windows-only (CET Lua, s'exécute au chargement du jeu). Conçu/écrit sur macOS,
testé en jeu.

## Structure

| Élément | Rôle |
|---|---|
| `overlay/bin/x64/plugins/cyber_engine_tweaks/mods/TesseraAutoLoad/init.lua` | hook `SingleplayerMenuGameController.OnSavesForLoadReady` → charge la save ciblée par nom |
| `save/` | **le fichier de sauvegarde lui-même — PAS COMMITÉ pour l'instant** (voir ci-dessous) |

## État (2026-07-06)

- **Hook d'auto-chargement** : codé, sourcé (2 mods réels indépendants : `Nats-ji/CP77-Skip-Main-Menu`
  CET Lua, `psiberx/cp2077-playground` redscript ; méthode `LoadSaveInGame` confirmée par dump
  RTTI local). **Jamais testé en jeu.**
- **Fichier de sauvegarde** : Lucas a téléchargé "Skip to Act 2 Save and more" (Nexus #27436),
  variante `1_Act 2 Playing for Time` (`ManualSave-20`) — `buildPatch: 2.3`, très proche de notre
  cible v2.31 (contrairement à un premier essai en v1.12, incompatible, écarté). **Pas encore
  committé** : c'est un fichier tiers (contenu d'un autre modder), et `tessera-core` est destiné à
  un mirror public — vérification des conditions de redistribution du mod en cours avant de
  décider où/si ce fichier précis vit dans le dépôt. En attendant il reste local
  (`tessera-core/client-mod/tessera-playtest-save/save/`, hors git).
- **Alternative plus propre** : si la vérification de licence bloque, Lucas rejoue lui-même
  jusqu'au point de spawn voulu sur sa propre install v2.31 et fournit sa propre sauvegarde —
  aucune question de redistribution dans ce cas.

## À faire ensuite

- Tester le hook en jeu une fois qu'une sauvegarde valide (licence ok ou save perso) est en place.
- Câbler côté launcher : la sauvegarde doit être copiée vers
  `%USERPROFILE%\Saved Games\CD Projekt Red\Cyberpunk 2077\<slot>\` (chemin Windows spécial, PAS
  la racine du jeu) — mécanisme différent du zip d'overlay générique utilisé par les autres
  packages du modset, pas encore câblé côté `tools/release`/launcher.
- Ajuster `TARGET_SAVE_NAME` dans `init.lua` si le nom réel du dossier de save diffère.

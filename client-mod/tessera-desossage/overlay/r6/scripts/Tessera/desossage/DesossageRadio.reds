module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Levier radios ambiantes (bornes de rue, radios d'appartement/boombox) — demande de Lucas
// (2026-07-19 : « les radios aussi », dans le cadre de la ville vide).
//
// MÉCANISME (2026-07-19) : fact save `radio_on`, posé via QuestsSystem.SetFact — MÊME mécanisme
// que `tutorials` (disable_tutorials) et `airTraffic` (air_traffic_off), tous deux déjà en place.
// La famille de facts `radio_on`/`tv_on` est citée en EXEMPLE par le wiki officiel RedModding
// (référencée dans le commentaire de Tessera_ApplyTutorials, DesossageEvents.reds) — d'où le
// choix de ce nom. Polarité PROPRE au fact : `radio_on` = 1 (allumé) / 0 (éteint), donc
// l'inverse de `air_traffic_off` (qui est un fact « off »). Ne pas se tromper de sens.
//
// POURQUOI SetFact et pas un @wrapMethod(Radio) : un override redscript avec une signature/
// visibilité devinée qui échoue ferait tomber TOUT r6/scripts (désossage, uikit, Cyberverse) —
// jeu qui ne démarre plus (piège documenté dans CLAUDE.md / README du module). SetFact est une
// méthode native à signature connue (n"...", Int32) : compile garantie. On livre donc d'abord la
// version compile-safe, on teste l'EFFET en jeu, on itère si besoin (cf. fallback ci-dessous).
//
// PIN IN-GAME : jamais testé — HYPOTHÈSE double : (1) que `radio_on` soit bien un fact GLOBAL qui
// coupe les radios ambiantes (et pas juste l'état d'une radio précise / la radio de poche du
// joueur) ; (2) que la polarité soit bien on=1/off=0. À CONFIRMER en jeu : plus de musique aux
// bornes de rue / dans les appartements. Si aucun effet → activer le fallback device ci-dessous.
public func Tessera_ApplyRadios(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  // e.active = radios vanilla ACTIVES (radio_on=1) ; défaut false = monde vide = radio_on=0.
  let value: Int32 = 0;
  if e.active { value = 1; }
  GameInstance.GetQuestsSystem(game).SetFact(n"radio_on", value);
  FTLog(s"[Tessera/Desossage] radios → fact radio_on=\(value) (SetFact)");
}

// ─────────────────────────────────────────────────────────────────────────────
// FALLBACK device (à activer SEULEMENT si le fact `radio_on` ci-dessus n'a aucun effet en jeu).
//
// Les bornes de rue / boombox sont des devices `Radio` (classe `Radio extends InteractiveDevice`,
// confirmée au dump RTTI local — tools/nativedb/search.py show Radio). `TurnOffDevice()`,
// `DeactivateDevice()` et `CutPower()` y existent ; `ResolveGameplayState()` est déclaré EN PROPRE
// sur `Radio` (pas seulement hérité — vérifié sans --deep) et tourne à l'initialisation du device,
// donc c'est un bon point d'accroche pour couper l'alimentation au spawn.
//
// AVANT DE DÉ-COMMENTER : vérifier la signature EXACTE (visibilité + type de retour) de
// ResolveGameplayState sur `Radio` dans le script décompilé officiel
// (CDPR-Modding-Documentation/Cyberpunk-Scripts) — PAS le dump RTTI, qui ne donne pas la
// visibilité redscript (piège récurrent documenté). Une mauvaise signature ici casse TOUT
// r6/scripts. Le corps ci-dessous suppose `protected func ... -> Void` (à confirmer).
//
// @wrapMethod(Radio)
// protected func ResolveGameplayState() -> Void {
//   wrappedMethod();
//   if !DesossageSystem.GetLiveConfig(GetGameInstance()).radios.active {
//     this.TurnOffDevice();
//   }
// }

module Tessera.UiKit.Test

// ─────────────────────────────────────────────────────────────────────────────
// Harnais de test UI — bascule un écran candidat du florilège SANS rebuild, depuis la console
// CET. Voir docs/superpowers/specs/2026-07-14-ui-test-harness-design.md (D-H1).
// Usage (console CET, touche par défaut ~) :
//   Game.GetPlayer():Tessera_UiTest("phone", true)
//   Game.GetPlayer():Tessera_UiTest("phone", false)
// Noms valides : "phone", "radio", "radial", "walkie", "devconsole", "kitchensink".
// Chaque écran est un STUB documenté (UiTest<Écran>.reds) tant que sa méthode d'ouverture
// réelle n'a pas été confirmée en jeu (palier H1) — cf. protocole D-H2 de la spec.
//
// Piège déjà rencontré sur ce projet (DesossageConsole.reds, dev12→dev13) : `name == "police"`
// sur des String échoue en compilation (NO_MATCHING_OVERLOAD). Utiliser StrCmp(a, b) == 0.
// ─────────────────────────────────────────────────────────────────────────────

@addMethod(PlayerPuppet)
public func Tessera_UiTest(name: String, open: Bool) -> Void {
  let game = this.GetGame();
  if StrCmp(name, "phone") == 0 {
    Tessera_UiTestPhone(game, open);
    return;
  }
  if StrCmp(name, "radio") == 0 {
    Tessera_UiTestRadio(game, open);
    return;
  }
  if StrCmp(name, "radial") == 0 {
    Tessera_UiTestRadial(game, open);
    return;
  }
  if StrCmp(name, "walkie") == 0 {
    Tessera_UiTestWalkie(game, open);
    return;
  }
  if StrCmp(name, "devconsole") == 0 {
    Tessera_UiTestDevConsole(game, open);
    return;
  }
  if StrCmp(name, "kitchensink") == 0 {
    Tessera_UiTestKitchenSink(game, open);
    return;
  }
  FTLog(s"[Tessera/UiTest] nom d'écran inconnu \"\(name)\" — valides : phone, radio, radial, walkie, devconsole, kitchensink");
}

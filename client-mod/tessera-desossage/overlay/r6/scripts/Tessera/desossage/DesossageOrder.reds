module Tessera.Desossage

// Leviers ordre public : police / système de recherche (wanted/MaxTac) + sécurité ambiante.
// Symboles 2.31 confirmés via source décompilée (core/systems/preventionSystem.swift).

// Police : la coupure est gérée en CONTINU par le @wrapMethod ci-dessous sur
// `PreventionSystem.CanPreventionReactToInput()` — c'est LE garde appelé à tous les points de gain
// d'étoiles (crimes). Piloté par `police.active` de la config, donc plus robuste qu'un appel
// one-shot que le jeu ré-activerait (logique free-areas).
public func Tessera_ApplyPolice(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  FTLog(s"[Tessera/Desossage] police → gérée par hook CanPreventionReactToInput");
}

public func Tessera_ApplyAmbientSecurity(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // PIN IN-GAME : désactiver les devices de sécurité ambiants (tourelles/drones).
  FTLog(s"[Tessera/Desossage] (stub) sécurité ambiante → coupée");
}

// Garde persistant : la prévention ne réagit plus au joueur quand la config coupe la police.
@wrapMethod(PreventionSystem)
public final const func CanPreventionReactToInput() -> Bool {
  let sys: ref<DesossageSystem> = DesossageSystem.Get(this.GetGameInstance());
  if IsDefined(sys) && !sys.GetConfig().police.active {
    return false;
  };
  return wrappedMethod();
}

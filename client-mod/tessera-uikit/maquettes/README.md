# Maquettes HUD — kit et workflow

Dossier de **conception visuelle** des écrans HUD/UI de Tessera. On maquette en **HTML/CSS** (que je
génère et que tu ouvres dans un navigateur, sur Mac) pour décider du look **avant** de coder en
redscript ink. Le kit fixe un langage visuel commun pour que toutes les maquettes soient cohérentes.

## Ouvrir le kit

Ouvre **`tessera-hud-kit.html`** dans un navigateur. Tu y trouves : une **maquette d'écran de jeu**
(disposition cible du palier 2), une **galerie de composants** (barre de vie, jauges faim/soif,
bouton H2, roue radiale, toast, panneau), la **palette** et la **typo**. La police Rajdhani se charge
depuis Google Fonts (il faut internet ; sinon repli sur une condensée système).

## Ce que je peux générer (esthétiquement)

- **Des maquettes HTML/CSS fidèles à l'esthétique Cyberpunk** : cadres chanfreinés (clip-path),
  scanlines, jaune #FCEE0A / cyan #00E5FF signature, typo condensée Rajdhani + mono pour les
  chiffres. Statiques ou avec un peu d'interactivité (hover, états).
- **Vite, et en itérant** : je pars des tokens du kit → chaque nouvelle maquette est cohérente avec
  les autres et avec le panneau H2 déjà validé en jeu.
- **Plusieurs variantes d'un même écran** côte à côte pour comparer.
- **La traduction ensuite en redscript ink** : les couleurs/polices du kit correspondent à ce qu'on
  sait faire côté Codeware (`SetTintColor`, `SetFontFamily raj`, `clip`/nine-slice), donc une maquette
  validée devient une spec d'implémentation directe.
- **Publication en Artifact** si tu veux un lien à ouvrir/partager sans fichier — demande-le.

## Ce que je ne peux PAS (honnêteté)

- **Pas de rendu ink réel** : une maquette HTML est un **proxy de design**, pas le jeu. Le pixel-perfect
  final se vérifie en jeu (Windows). Certaines choses (nine-slice exact d'un atlas natif, animations
  ink) s'approchent en CSS mais ne sont pas identiques.
- **Aucun asset CDPR** : on n'extrait/redistribue rien du jeu. Le kit est 100 % reconstruction CSS
  (police libre Rajdhani). Réutiliser un vrai atlas natif se fait **en jeu** via son depot path
  (`SetAtlasResource`), jamais en copiant le fichier.
- **Pas de HUD interactif jouable** ici : c'est de la maquette, l'input réel vit dans le jeu.

## Workflow

1. **Tu décris** (ou croques) un écran : « le HUD de mort », « la roue radiale à 3 étages »,
   « l'inventaire refondu »…
2. **Je génère une maquette** HTML avec les tokens du kit (souvent 2-3 variantes).
3. **Tu ouvres sur Mac, tu réagis**, on itère jusqu'à figer.
4. **Je traduis en redscript ink** sur le socle H2 (Codeware `InGamePopup`/factories), on compile-check
   en local (scc, sans lancer le jeu) puis on teste en jeu.

## Lien avec l'inventaire

Chaque écran vient de l'inventaire figé : `docs/superpowers/plans/2026-07-22-inventaire-ecrans-ui.md`
(Core d'abord). On maquette dans l'ordre de priorité décidé là-bas.

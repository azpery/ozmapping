# OZ Mapping

Logiciel de vidéo-mapping live en Rust : calques vidéo superposés, formes de mapping complexes, effets et presets en temps réel, projection sur un second écran.

## Télécharger

Les versions compilées sont sur la page [Releases](https://github.com/azpery/ozmapping/releases/latest) :

- **Windows** : `OZ-Mapping-<version>-windows-x64.zip`. Dézippez puis lancez `OZ Mapping.exe` ; ffmpeg est inclus.
- **macOS** (Apple Silicon et Intel) : `OZ-Mapping-<version>.dmg`. Glissez l'app dans Applications. La première fois, faites clic droit sur l'app puis « Ouvrir » : elle n'est pas signée par un compte Apple Developer. ffmpeg doit être installé : `brew install ffmpeg`.

Le paquet Windows contient ffmpeg, sous licence GPL (fichier `ffmpeg/LICENSE.txt`). Il s'agit des builds [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), dont le code source est disponible sur [ffmpeg.org](https://ffmpeg.org/download.html) et sur ce dépôt.

## Prérequis

- Rust (stable)
- ffmpeg et ffprobe : `brew install ffmpeg` sur macOS, `apt install ffmpeg` sur Linux

## Lancer

```sh
cargo run --release
cargo run --release -- clip1.mp4 clip2.mov   # ajoute les vidéos et lance la première
cargo run --release -- spectacle.ozmap       # ouvre un projet
```

## Application macOS

```sh
scripts/build-macos.sh              # Mac de cette architecture (Apple Silicon ici)
scripts/build-macos.sh --universal  # Apple Silicon + Intel
```

Le script produit `dist/OZ Mapping.app` (avec son icône) et `dist/OZ-Mapping-<version>.dmg`. L'app reçoit une signature ad hoc, pas une signature Apple Developer : sur un autre Mac, il faut faire la première fois clic droit sur l'app puis « Ouvrir ». ffmpeg doit être installé sur la machine (`brew install ffmpeg`).

## Windows

Le build Windows se fait sur GitHub Actions (workflow « Build ») : onglet **Actions** > **Build** > **Run workflow**, ou en poussant un tag `v*`, qui publie aussi une release. Le fichier `OZ-Mapping-<version>-windows-x64.zip` contient `OZ Mapping.exe` et ffmpeg (dans `ffmpeg/bin`, build GPL de [BtbN](https://github.com/BtbN/FFmpeg-Builds)) : il suffit de dézipper et de lancer l'exe. Le même workflow produit le .dmg macOS universel.

## Utilisation

**Médias.** Bouton « + Ajouter… » ou glisser-déposer (mp4, mkv, mov, avi, webm…). Chaque média a une miniature.
- **Clic** : pré-visualisation dans le moniteur de gauche, sans toucher à la sortie.
- **Double-clic**, **GO**, touches **1-9** ou **← →** : envoie le média sur le calque sélectionné, avec un fondu enchaîné.

**Sortie.** La fenêtre « Sortie » s'ouvre au démarrage. Choisissez l'écran du projecteur dans le menu **Écran** de la barre du haut : la sortie s'y place en plein écran. L'écran choisi est retenu pour les prochains lancements. Sinon, déplacez la fenêtre à la main et appuyez sur **F** ou double-cliquez dessus pour le plein écran.

**Calques** (panneau du bas, jusqu'à 8). Chaque calque a sa vidéo, son opacité, son mode de fusion (Normal, Addition, Écran, Produit, Éclaircir, Assombrir), ses effets et sa forme. Le calque du haut de la liste est au premier plan. **Tab** passe au calque suivant.

**Effets** (onglet Effets). Ils s'appliquent au calque sélectionné.
- 26 presets intégrés : Néon, Kaléido, Mandala, Miroir 4, Tunnel, Rétro VHS, Glitch, 8-bit, Pop art, Arc-en-ciel, Psyché, Rêve, Noir & blanc, Duotone rose, Bleu glacé, Négatif, Strobe, Vortex, Tunnel acide, Liquide, Acide, Mosaïque, Écho, Solarisé, Respiration, Trip.
- Section **Psychédélique** : tourbillon, tunnel (coordonnées polaires qui défilent), mosaïque miroir, distorsion liquide, palette acide, arc-en-ciel spatial, solarisation, pulsation (zoom qui respire), écho (copies superposées).
- « Aléatoire » tire une combinaison au hasard.
- **Revenir sans effet** : « Aucun » (touche **0**) retire les effets du calque sélectionné et coupe son mode AUTO. « Sans effets » dans la barre du haut (touche **E**) affiche toutes les vidéos brutes, sur tous les calques, sans perdre les réglages : un second appui les rétablit.
- La durée de « Transition » règle le passage en douceur d'un preset à l'autre.
- « Mémoriser » crée vos propres presets (clic droit pour en supprimer un).
- **AUTO** (touche **A**) : enchaîne automatiquement des effets tirés au hasard sur le calque sélectionné, sans répéter le même deux fois de suite. Un **thème** oriente les choix : Tout, Psychédélique, Couleur (sans déformer l'image), Vitesse, Géométrie, Glitch, Doux ; l'**intensité** dose les effets générés. On règle aussi l'intervalle entre deux changements, la durée du fondu et, pour le thème Tout, la part d'**effets inédits** : le pourcentage de changements qui inventent un nouvel effet au lieu de reprendre un preset (25 % par défaut ; « Mémoriser » permet de garder un effet inventé qui plaît). Le stroboscope est exclu par défaut. « Suivant » force un changement immédiat ; choisir un preset à la main le garde affiché un intervalle complet. Chaque calque a son propre mode Auto, enregistré dans le projet.

**Mix auto** (onglet Mix) : cochez plusieurs vidéos (dans l'onglet ou avec l'icône de bobine sur chaque média), puis activez MIX AUTO. Le calque enchaîne alors des séquences de durée aléatoire : changement de vidéo ou saut à un autre moment, lecture à l'endroit ou **à l'envers**, vitesse variable, coupe franche ou fondu. Chaque réglage (durées, probabilités, vitesses, fondus) est ajustable et enregistré dans le projet ; chaque calque peut avoir son propre mix.

**Lecture à l'envers** : bouton dans le panneau Lecture, ou touche **R**. La vidéo est décodée par petits morceaux de la fin vers le début (en 1280×720 au maximum).

**Forme** (onglet Forme, touche **M** pour éditer sur l'aperçu ou directement sur la sortie) :
- **Déformation** : une grille de 1×1 (4 coins en perspective) à 8×8 points pour suivre des surfaces courbes. « Aplatir » retire les courbures.
- **Masque** : un polygone (rectangle, cercle, triangle, hexagone, étoile, ou forme libre) avec adoucissement des bords et inversion. Double-clic sur un bord pour ajouter un point, clic droit sur un point pour le supprimer.
- **Masque SVG** : « Importer un SVG… » (ou glisser-déposer un `.svg` sur la fenêtre) reprend la forme d'un logo ou d'un pictogramme comme masque du calque sélectionné : plusieurs formes, trous, courbes et traits sont pris en charge. La forme est centrée sur la sortie, simplifiée (2 000 points au maximum), et ses points restent éditables. Les textes et images contenus dans le SVG sont ignorés : convertissez les textes en tracés dans votre logiciel de dessin.
- **Masque depuis une image** : « Depuis une image… » (ou glisser-déposer un PNG, JPG, WebP, BMP ou TIFF) détoure l'image et en fait un masque vectoriel éditable. La forme vient de la transparence si l'image en a, sinon de ce qui se détache du fond (logo noir sur blanc ou blanc sur noir, silhouette…) ; les trous sont conservés et les petites taches ignorées. L'image entière est ajustée à la sortie : prenez une photo du support depuis le projecteur, détourez-la (fond noir, sujet blanc) et le masque tombe pile dessus. « Inverser » garde l'extérieur au lieu de la forme.
- **Maj** permet un réglage précis, la croix déplace toute la forme et **G** affiche une mire.

**Rythme** (onglet Rythme) : les effets réagissent au son ou à un tempo.
- **Source** : une entrée audio (micro, carte son) ou un tempo tapé. Pour le son joué par l'ordinateur lui-même, passez par une entrée virtuelle (BlackHole sur Mac, « Mixage stéréo » sous Windows). Le tempo se donne en tapant en rythme sur **TAP** (barre du haut) ou la touche **T**. On peut aussi le saisir, ou le diviser et le doubler.
- **Bande** (entrée audio) : les temps sont détectés dans les **basses** (par défaut : grosse caisse et basse, le plus franc et le plus réactif), sur **tout** le son ou dans les **aigus** (charleston, cymbales). L'analyse se fait toutes les 5 ms, avec un petit tampon audio pour limiter la latence.
- **Tap tempo** : chaque tap donne un temps immédiatement ; le BPM s'affine au fil des taps.
- **Réactions toutes prêtes** : Pulsation, Club, Stroboscope, Glitch, Rotation, Miroir, Négatif, Onde de choc, Kaléido, Arc-en-ciel, Douceur.
- **Réaction du calque**, à doser :
  - sur chaque temps : coup de zoom, flash, secousse RVB, saut de teinte, négatif, stroboscope (noir entre les temps), glitch, onde de choc, kaléidoscope, rotation, miroir un temps sur deux ;
  - avec l'intensité de la bande choisie : déformations et saturation qui s'amplifient.
- **Enchaînements sur les temps** : le mode AUTO et le mix auto peuvent changer tous les N temps au lieu d'une durée en secondes.

**Jauge CPU** (en haut à droite) : la charge de l'app et de ses décodeurs ffmpeg, en pourcentage de la machine (vert, orange puis rouge). La partie grise montre la charge du reste de la machine. Le survol donne le détail (nombre de décodeurs, de cœurs).

**Projet** (menu Projet). Les fichiers `.ozmap` sont du JSON : médias, calques (avec leur vidéo), effets, formes, presets et réglages. Les médias rangés dans le dossier du projet sont enregistrés en chemins relatifs, ce qui permet de déplacer le dossier. Un « * » dans le titre signale des modifications non enregistrées. La session en cours est aussi restaurée automatiquement au lancement.

| Raccourci | Action |
|-----------|--------|
| Cmd+N / Cmd+O (Ctrl sous Windows) | Nouveau projet / ouvrir un projet |
| Cmd+S / Maj+Cmd+S (Ctrl sous Windows) | Enregistrer / enregistrer sous |
| Espace | Pause / lecture du calque sélectionné |
| 1-9, ← → | Lancer un média sur le calque sélectionné |
| Tab | Calque suivant |
| T | Tap tempo (BPM) |
| R | Lecture à l'envers / à l'endroit (calque sélectionné) |
| 0 | Retirer les effets du calque sélectionné (et couper AUTO) |
| E | Tout afficher sans effets (contournement, réglages conservés) |
| A | Mode Auto (effets aléatoires) sur le calque sélectionné |
| B | Blackout progressif |
| G | Mire de calibration |
| M | Édition de la forme |
| F / Échap | Plein écran de la sortie / quitter le plein écran |

## Architecture

- `src/video.rs` : décodage via un processus `ffmpeg` qui produit des images RGBA, cadencées par un thread (vitesse, pause, seek, boucle). Extraction des miniatures.
- `src/renderer.rs` : composition OpenGL des calques (modes de fusion, fondus), rendu d'un maillage déformé et masque polygonal dans le shader, effets.
- `src/surface.rs` : grille de déformation (perspective et splines de Catmull-Rom), masques, poignées d'édition.
- `src/mask.rs` : import SVG et détourage d'images (carrés marchants) des masques, rastérisation (tiny-skia) en texture.
- `src/rhythm.rs` : écoute audio (cpal), détection des temps dans les graves, tap tempo, réactions des calques.
- `src/screens.rs` : liste des écrans pour placer la sortie.
- `src/automix.rs` : tirage des séquences du mix auto.
- `src/effects.rs` : paramètres d'effets, interpolation, presets intégrés et aléatoires.
- `src/project.rs` : fichier projet `.ozmap`.
- `src/thumbs.rs` : génération des miniatures en arrière-plan.
- `src/cpu.rs` : mesure de la charge CPU (app + ffmpeg, machine).
- `src/theme.rs`, `src/icons.rs` : apparence (palette, composants) et icônes [Phosphor](https://phosphoricons.com) (police dans `assets/`, licence MIT).
- `src/app.rs` : interface egui (fenêtre de contrôle et fenêtre de sortie).

Limites : pas de son ; les vidéos sont décodées en 1920×1080 au maximum.

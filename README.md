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

**Sortie.** La fenêtre « Sortie » s'ouvre au démarrage. Déplacez-la sur le projecteur, puis appuyez sur **F** ou double-cliquez dessus pour le plein écran.

**Calques** (panneau du bas, jusqu'à 8). Chaque calque a sa vidéo, son opacité, son mode de fusion (Normal, Addition, Écran, Produit, Éclaircir, Assombrir), ses effets et sa forme. Le calque du haut de la liste est au premier plan. **Tab** passe au calque suivant.

**Effets** (onglet Effets). Ils s'appliquent au calque sélectionné.
- 17 presets intégrés : Néon, Kaléido, Mandala, Miroir 4, Tunnel, Rétro VHS, Glitch, 8-bit, Pop art, Arc-en-ciel, Psyché, Rêve, Noir & blanc, Duotone rose, Bleu glacé, Négatif, Strobe.
- « Aléatoire » tire une combinaison au hasard.
- **Revenir sans effet** : « Aucun » (touche **0**) retire les effets du calque sélectionné et coupe son mode AUTO. « Sans effets » dans la barre du haut (touche **E**) affiche toutes les vidéos brutes, sur tous les calques, sans perdre les réglages : un second appui les rétablit.
- La durée de « Transition » règle le passage en douceur d'un preset à l'autre.
- « Mémoriser » crée vos propres presets (clic droit pour en supprimer un).
- **AUTO** (touche **A**) : enchaîne automatiquement des effets tirés au hasard sur le calque sélectionné, sans répéter le même deux fois de suite. On règle l'intervalle entre deux changements, la durée du fondu et la part d'**effets inédits** : le pourcentage de changements qui inventent un nouvel effet au lieu de reprendre un preset (25 % par défaut ; « Mémoriser » permet de garder un effet inventé qui plaît). Le stroboscope est exclu par défaut. « Suivant » force un changement immédiat ; choisir un preset à la main le garde affiché un intervalle complet. Chaque calque a son propre mode Auto, enregistré dans le projet.

**Forme** (onglet Forme, touche **M** pour éditer sur l'aperçu ou directement sur la sortie) :
- **Déformation** : une grille de 1×1 (4 coins en perspective) à 8×8 points pour suivre des surfaces courbes. « Aplatir » retire les courbures.
- **Masque** : un polygone (rectangle, cercle, triangle, hexagone, étoile, ou forme libre) avec adoucissement des bords et inversion. Double-clic sur un bord pour ajouter un point, clic droit sur un point pour le supprimer.
- **Maj** permet un réglage précis, la croix déplace toute la forme et **G** affiche une mire.

**Jauge CPU** (en haut à droite) : la charge de l'app et de ses décodeurs ffmpeg, en pourcentage de la machine (vert, orange puis rouge). La partie grise montre la charge du reste de la machine. Le survol donne le détail (nombre de décodeurs, de cœurs).

**Projet** (menu Projet). Les fichiers `.ozmap` sont du JSON : médias, calques (avec leur vidéo), effets, formes, presets et réglages. Les médias rangés dans le dossier du projet sont enregistrés en chemins relatifs, ce qui permet de déplacer le dossier. Un « * » dans le titre signale des modifications non enregistrées. La session en cours est aussi restaurée automatiquement au lancement.

| Raccourci | Action |
|-----------|--------|
| Cmd+N / Cmd+O (Ctrl sous Windows) | Nouveau projet / ouvrir un projet |
| Cmd+S / Maj+Cmd+S (Ctrl sous Windows) | Enregistrer / enregistrer sous |
| Espace | Pause / lecture du calque sélectionné |
| 1-9, ← → | Lancer un média sur le calque sélectionné |
| Tab | Calque suivant |
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
- `src/effects.rs` : paramètres d'effets, interpolation, presets intégrés et aléatoires.
- `src/project.rs` : fichier projet `.ozmap`.
- `src/thumbs.rs` : génération des miniatures en arrière-plan.
- `src/cpu.rs` : mesure de la charge CPU (app + ffmpeg, machine).
- `src/theme.rs`, `src/icons.rs` : apparence (palette, composants) et icônes [Phosphor](https://phosphoricons.com) (police dans `assets/`, licence MIT).
- `src/app.rs` : interface egui (fenêtre de contrôle et fenêtre de sortie).

Limites : pas de son ; les vidéos sont décodées en 1920×1080 au maximum.

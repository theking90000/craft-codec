# Première spécification de CRAFT I/O

Proposition du 20 septembre 2026, révisée après discussion des offsets, des
nonces et du choix d'une API unique. Ce document fixe un périmètre et un plan
de développement. Les signatures sont indicatives ; aucun codec n'est encore
implémenté et aucune performance n'est mesurée. L'API entièrement synchrone
est la recommandation actuelle, encore à confirmer par l'utilisateur.

Il s'appuie sur la conversation « Utilité du chunked HTTP/1.1 », le besoin
exprimé pour `craft-io`, et la structure actuelle de `../carbon-io`.

## 1. Objectif et priorités

`craft-io` transforme une suite de trames en une représentation stockable, et
effectue la transformation inverse. Chaque trame est décodable indépendamment.
La lecture peut commencer à une frontière de trame, puis reste séquentielle.

```text
Écriture : trame brute → compression facultative → chiffrement facultatif → octets
Lecture  : octets → authentification/déchiffrement → décompression → trame brute
```

Les priorités sont, dans cet ordre :

1. Performance.
2. Simplicité du hot path.
3. Robustesse.
4. Simplicité de l'API.
5. Généralité.

Recommandation révisée : une seule API publique, synchrone, qui travaille sur
des buffers. CRAFT n'accepte ni `Read` ni `AsyncRead` et n'expose aucun `poll_*`.
L'intégrateur choisit ses I/O et appelle CRAFT lorsqu'une trame est disponible.
Une application synchrone et une application async utilisent les mêmes méthodes.

## 2. Responsabilités

CRAFT connaît les frontières de trames, leurs tailles, les offsets physiques,
les transformations et les métadonnées nécessaires au décodage.

L'appelant fournit et conserve :

- les trames brutes à écrire ;
- l'accès aux octets stockés, notamment `open(offset, limit)` ;
- une clé secrète si le chiffrement est activé ;
- le stockage fiable des métadonnées et leur association au bon objet ;
- la publication atomique de l'objet et de ses métadonnées après succès.

Le scheduling, les retries, le cache, les tâches, les pools de buffers partagés,
HTTP, TLS, les fichiers et les services de gestion de clés restent à l'extérieur.
Les lectures I/O peuvent couper ou réunir des trames ; leurs frontières n'ont
aucune signification pour CRAFT.

La première version traite des objets immuables, finalisés avant lecture.
La lecture pendant la construction d'un index et la modification de trames
existantes demanderaient d'autres contrats.

Avec CARBON, une adaptation externe présente un objet ou une vue CRAFT comme
un `ReadFile<T>` ou un `WriteFile<T>`. Cet adaptateur possède le reader ou writer
I/O, assemble les trames et appelle CRAFT. CARBON organise les différents
adaptateurs. CRAFT ne dépend pas de CARBON.

## 3. Modes et dimensions indépendantes

Il faut distinguer la taille logique avant transformation de la taille physique.
Une compression indépendante par trame rend généralement la seconde variable.

| Profil | Trames logiques | Compression | Chiffrement | Tailles à conserver par trame |
| --- | --- | --- | --- | --- |
| 1 | Fixes | Non | AES-256-GCM | Aucune, sauf dernière trame à part |
| 2 | Variables | Non | AES-256-GCM | Taille logique |
| 3 | Fixes | Si plus petit | AES-256-GCM | Taille du payload stocké |
| 4 | Variables | Si plus petit | AES-256-GCM | Taille logique et taille du payload |
| 5 | Fixes ou variables | Si plus petit | Non | Comme 3 ou 4 |
| 6 | Fixes ou variables | Non | Non | Comme 1 ou 2 |

Les deux derniers profils sont des familles. En autorisant les deux découpages,
on obtient huit combinaisons avec les trois axes fixe/variable,
compression/identité et chiffrement/identité. Il n'est pas nécessaire d'écrire
huit implémentations de reader.

Chaque objet choisit son découpage, son codec de compression et son chiffrement
une seule fois. Seule la décision de conserver la compression varie par trame.

## 4. Format des données

Pour la trame `i`, on note :

```text
L[i] = nombre d'octets bruts
P[i] = nombre d'octets après compression éventuelle, hors tag
T    = 16 avec AES-GCM, 0 sans chiffrement
S[i] = P[i] + T
O[i] = somme de S[j] pour j < i
```

Une trame stockée contient exactement :

```text
[payload de P[i] octets][tag de 16 octets si chiffré]
```

Le payload est chiffré si nécessaire. Le flux est la concaténation de ces
trames, sans padding, en-tête par trame, IV stocké ni marqueur de fin.
Le descripteur externe est indispensable pour le lire.

Pour un découpage fixe de taille `F`, toutes les trames sauf éventuellement
la dernière ont `L[i] = F`. Sans compression :

```text
O[i] = i × (F + T)
taille_totale = (N - 1) × (F + T) + taille_dernière + T, si N > 0
```

Le dernier bloc peut être plein. Un objet vide contient zéro trame et zéro
octet. Une trame vide est interdite, y compris en mode variable.

### Compression seulement si rentable en octets

On conserve le résultat complet du codec seulement si sa taille est strictement
inférieure à `L[i]`. Sinon, on conserve les octets bruts. Le tag a la même
taille dans les deux cas et ne change pas la comparaison.

Cela permet de déduire la décision sans flag supplémentaire :

```text
P[i] < L[i] : payload compressé
P[i] = L[i] : payload brut
P[i] > L[i] : métadonnées invalides
```

Sans codec configuré, seule l'égalité est valide. À la lecture, on authentifie
avant de décompresser et on exige exactement `L[i]` octets en sortie.
Le décodeur doit aussi refuser un payload compressé non entièrement consommé.

Cette économie dépend de ces invariants. Accepter plus tard de la compression
qui agrandit les données imposerait un changement de format.

La rentabilité en octets ne garantit pas un gain de temps total. Les benchmarks
devront mesurer aussi le coût des tentatives infructueuses de compression.

## 5. Métadonnées et index

Le descripteur externe contient la version du format, le découpage, les codecs,
le nombre de trames, la limite de taille brute et les données suivantes :

- découpage fixe : taille `F` et taille de la dernière trame ;
- découpage variable : tableau des tailles logiques, dernière trame incluse ;
- compression activée : tableau des tailles de payload, hors tags.

La taille de la dernière trame est donc toujours disponible à part du flux.
En variable, elle est déjà la dernière entrée de l'index ; inutile de la
dupliquer. Pour un objet vide, elle est absente.

Les longueurs totales sont calculables. Si le stockage externe les conserve
aussi, CRAFT vérifie leur cohérence à la préparation du descripteur.
La clé est un argument secret séparé, pas un champ sérialisable du descripteur.

### Largeurs automatiques

Recommandation : encoder les longueurs positives sous la forme `longueur - 1`.
Le type est choisi automatiquement d'après la taille maximale configurée `M`.

| Taille maximale positive | Type des entrées |
| --- | --- |
| `M <= 2^16` | `u16` |
| `2^16 < M <= 2^32` | `u32` |
| `2^32 < M <= u64::MAX` | `u64` |

Ainsi, 65 536 octets tiennent dans une entrée `u16` de valeur 65 535.
Avec des longueurs directes, 65 536 nécessiterait déjà `u32`.

Une largeur unique par tableau suffit. Aucun varint ni enum par entrée.
Le découpage variable exige lui aussi un maximum explicite, utile pour borner
la mémoire et choisir la largeur avant le hot path. Dépasser cette limite est
une erreur ; l'élargissement automatique correspond au choix d'une configuration
plus grande, pas à une réallocation surprise au milieu d'une trame.

Un champ `u64` n'autorise pas à allouer une trame de cette taille. Les limites
mémoire, celles du codec et les conversions vers `usize` restent vérifiées.
Les offsets et totaux utilisent `u64` avec additions et multiplications checked.
Décoder `longueur - 1` se fait après élargissement ; `u64::MAX` comme entrée est
invalide puisqu'ajouter un dépasserait `u64`.

### Calcul initial, puis parcours séquentiel

Pour commencer à la trame `k`, sommer une fois les tailles physiques des
trames précédentes. Le parcours lit uniquement les métadonnées, aucun payload.

```text
offset = somme(P[j] + T), pour j < k
backend.open(offset)
puis consommation séquentielle de P[i] + T octets par trame
```

Le coût du positionnement est O(k), au pire O(N), avec O(1) mémoire auxiliaire.
Chaque progression suivante coûte O(1). Il suffit de conserver l'indice courant
et, si utile à l'appelant, l'offset cumulé courant. Le reader backend avance
déjà lui-même lorsqu'il consomme les octets.

Aucun tableau d'offsets cumulés ni points de repère supplémentaires.
La proposition précédente optimisait les ouvertures arbitraires répétées au
prix de mémoire additionnelle ; ce n'est pas nécessaire au contrat retenu.
Le profil fixe sans compression garde son calcul direct en O(1).

Pour une plage logique bornée, connaître aussi la fin physique avant l'ouverture
demande de parcourir les tailles jusqu'à la dernière trame sélectionnée.
Si celle-ci porte l'indice `j`, la préparation coûte O(j + 1), au pire O(N),
toujours avec O(1) mémoire auxiliaire. L'itérateur reparcourt ensuite uniquement
les entrées sélectionnées, en O(1) par trame. Ce second parcours des métadonnées
évite de matérialiser un tableau de descripteurs. Le cas fixe sans compression
calcule directement les deux frontières. Les totaux déjà validés permettent
d'obtenir directement la fin physique d'une sélection allant jusqu'à EOF.

L'index compact reste nécessaire pour connaître les tailles variables. Pour
1 GiB découpé en 64 KiB, ses 16 384 tailles de payload en `u16` occupent 32 KiB.
En variable, le tableau des tailles logiques s'ajoute. Le curseur n'en fait
pas de copie. Toute validation globale des métadonnées coûte séparément O(N)
et peut être effectuée une fois lors de leur chargement.

Le stockage externe peut utiliser sa propre sérialisation. CRAFT expose des
types de métadonnées et valide leur contenu ; ni Serde ni format de conteneur
général ne sont nécessaires au départ.

## 6. Contrat cryptographique

AES-256-GCM avec nonce de 96 bits et tag complet de 128 bits constitue le
premier profil. Les nonces doivent être uniques sous une même clé. La
construction déterministe avec champs fixe et compteur est décrite dans
[NIST SP 800-38D, section 8.2.1](https://tsapps.nist.gov/publication/get_pdf.cfm?pub_id=51288).

Proposition pour CRAFT, sous réserve de revue avant de figer le format :

```text
Une clé indépendante de 256 bits par objet immuable.
nonce(i) = 0x00000000 || u64_be(i)
AAD      = vide dans le profil minimal, avec métadonnées fiables à part
```

Le numéro de trame est celui de l'objet, même quand la lecture commence au
milieu. Le codec peut recevoir explicitement cet indice, comme dans
`encode_frame(payload, local_frame_index)`. Le curseur de l'intégrateur gère
l'ordre ; aucun second compteur anti-réécriture n'est imposé dans le codec.

Le contrat est de ne jamais chiffrer deux payloads différents avec la même
clé et le même indice. Write-once permet de tenir ce contrat dans un objet,
mais n'empêche pas deux objets d'avoir chacun une trame zéro. D'où la clé
distincte par objet si l'indice est la seule donnée variable du nonce.

Les lectures répétées ne consomment pas de nonce. Retransmettre un ciphertext
identique est possible. Un réencodage strictement identique sous le même
couple clé/nonce reproduit le même résultat ; cela ne doit pas devenir une
autorisation de modifier le payload ou l'AAD. Si une tentative peut changer
les octets à chiffrer, elle nécessite une nouvelle clé. La bibliothèque ne
gère pas les retries et ne prétend pas vérifier l'unicité globale des clés.

Un AAD vide est compatible avec le modèle retenu : les métadonnées et leur
association à l'objet sont fiables, et la clé est dédiée à cet objet et à ce
profil. L'ajout d'un AAD liant version et tailles reste une extension possible,
pas une obligation pour corriger un risque de réutilisation de nonce.

On ne livre aucun octet déchiffré avant validation du tag. Une erreur
d'authentification arrête le reader. La substitution depuis un autre objet
échoue grâce à sa clé différente ; celle depuis une autre position grâce au
nonce attendu.

La disparition de trames finales entières se détecte par le nombre et les
tailles attendus dans les métadonnées fiables. Le tag d'une trame précédente
ne prouve pas que le suffixe existe. Un reader abandonné avant la fin ne
valide pas les trames qu'il n'a pas lues.

Avant implémentation crypto, fixer le maximum de trames et le volume maximal
par clé selon les tailles visées et l'objectif de sécurité. Un compteur de
64 bits ne signifie pas que chiffrer `2^64` trames sous une clé est acceptable.
La limite GCM par invocation est également indépendante du type de l'index.
La génération et le stockage des clés restent à l'appelant ; CRAFT applique
les limites retenues et ne journalise jamais la clé.

Les profils sans chiffrement n'ont aucune authentification. Une corruption de
même longueur peut passer inaperçue ; une décompression réussie ne prouve pas
l'intégrité. Les tailles de trames et de compression restent visibles dans
les profils chiffrés, puisqu'aucun padding n'est prévu.

## 7. API synchrone et lecture pilotée de l'extérieur

L'appelant demande une plage d'octets logiques. `Metadata` la traduit en une
plage physique couvrant les trames entières concernées et en un itérateur
empruntant l'index. Aucun `Vec<Frame>` ni cache d'offsets cumulés.

Les bornes sont demi-ouvertes, comme les slices Rust : `start` inclus,
`end` exclu. `None` signifie la fin logique de l'objet, sans sentinelle `-1`.
Proposition de signature :

```rust,ignore
fn range(
    &self,
    start: u64,
    end: Option<u64>,
) -> Result<(Range<u64>, FrameIter<'_>)>;

struct FrameRead {
    spec: FrameSpec,           // Indice objet, tailles brute et stockée.
    selected: Range<usize>,   // Portion à restituer après décodage complet.
}
```

Le premier `Range<u64>` est physique, relatif au début de la représentation
stockée de l'objet. L'itérateur produit des `FrameRead`. Les champs sont ici
illustratifs ; les descripteurs proviennent de métadonnées validées.
Offsets globaux et indices utilisent `u64`, les tailles de buffers et les
bornes de slices utilisent `usize` après conversion contrôlée.

```rust,ignore
// Code de l'intégrateur async, signatures indicatives.
let (query, frames) = metadata.range(50, Some(50_213))?;
if !query.is_empty() {
    let mut source = backend
        .open(query.start, Some(query.end - query.start))
        .await?;
    let mut buffer = Vec::new();
    for frame in frames {
        buffer.resize(frame.spec.stored_len(), 0);
        source.read_exact(&mut buffer).await?;
        decoder.decode_frame(frame.spec, &mut buffer)?;
        consume(&buffer[frame.selected]).await?;
    }
}
```

Le décodeur reçoit uniquement `FrameSpec`, jamais `selected`, `start` ou `end`.
Il authentifie et décode la trame entière. Le découpage logique appartient à
`Metadata` pour son calcul, puis à l'appelant pour la sélection de la slice.
Demander quelques octets d'une trame exige de lire son payload entier et son
tag éventuel. La tranche résultante n'est livrée qu'après décodage réussi.

Pour une trame couvrant `[B, B + L)` et une requête logique `[a, b)` qui
l'intersecte, les bornes locales sont :

```text
selected.start = max(a, B) - B
selected.end   = min(b, B + L) - B
```

Seules les trames ayant une intersection non vide sont émises. La première
et la dernière peuvent être partielles ; une même trame peut porter les deux
découpes. Les trames intérieures ont `selected = 0..raw_len`. L'indice du
codec reste celui dans l'objet d'origine et ne repart jamais de zéro à chaque
requête. Un offset physique par élément est inutile au parcours séquentiel.

Exemple avec une première trame pleine de 65 536 octets bruts :

| Profil | Query physique pour `50..50_213` | Portion décodée restituée |
| --- | --- | --- |
| Brut | `0..65_536` | `50..50_213` |
| AES-GCM sans compression | `0..65_552` | `50..50_213` |
| Compression et AES-GCM | `0..(P[0] + 16)` | `50..50_213` |

La plage physique comprend les tags. Une trame finale courte utilise sa vraie
taille, sans arrondi au multiple suivant de la taille nominale.

Le backend reste extérieur. Une API `open(offset, limit)` reçoit une limite
exprimée en nombre d'octets, donc `query.end - query.start`, pas une position
finale. Pour HTTP, la borne finale du header Range est inclusive : une plage
physique non vide `start..end` devient `bytes=start-(end - 1)`. L'adaptateur
HTTP gère cette conversion et vérifie que les octets retournés correspondent
à la plage demandée. Voir [RFC 9110, section 14.1.2](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.1.2).

Le contrat interne proposé est strict : `0 <= start <= end <= logical_len`
après résolution de `None`. Une borne hors objet ou inversée est une erreur,
sans clamp implicite. Une plage vide, y compris `EOF..EOF`, donne un itérateur
vide et une query vide dont la position n'a pas de signification pour les I/O.
Elle n'ouvre aucun backend. Le traitement des suffixes HTTP et des demandes
HTTP dépassant EOF reste à l'intégrateur avant l'appel à CRAFT.

Il n'est pas nécessaire de créer un protocole d'intentions `NeedRead`,
`NeedWrite` ou `Pending`. Le descripteur donne déjà le nombre exact d'octets
à lire. La gestion des I/O partielles appartient à l'adaptateur extérieur.
La même boucle peut utiliser un `Read` synchrone sans changer l'API de CRAFT.

Chaque lecture non vide consomme exactement
`P[i] + T` octets avant le décodage. Un EOF prématuré est une erreur, y compris
entre deux trames si la sélection en annonce encore.

La vue se termine après la dernière trame sélectionnée. L'adaptateur ne sonde
pas un octet supplémentaire, ce qui permet un objet embarqué dans un stockage
plus grand.
La vérification que l'objet backend ne contient aucun suffixe supplémentaire
reste au conteneur ou au backend. Ce contrat est différent du contrôle d'EOF
exact que CARBON applique à son flux de trames.

L'API de plage couvre aussi la lecture intégrale avec `range(0, None)`.
Le curseur par numéro de trame peut rester un détail interne tant qu'un
deuxième point d'entrée public n'est pas nécessaire.

### Propriété des buffers

Proposition minimale, avec des noms et types encore indicatifs :

```rust,ignore
// Modifie le buffer et retourne les tailles à enregistrer dans les métadonnées.
fn encode_frame(&mut self, index: u64, buffer: &mut Vec<u8>) -> Result<FrameSizes>;

// Remplace les octets stockés par les octets bruts vérifiés.
fn decode_frame(&mut self, spec: FrameSpec, buffer: &mut Vec<u8>) -> Result<()>;
```

`&mut self` permet de réutiliser le scratch de compression. Le contexte crypto
seul peut travailler à travers `&self` ; cela ne justifie pas d'imposer un
traitement concurrent au contexte de compression.

Sans compression, chiffrement et déchiffrement modifient le payload en place.
Prévoir la capacité du tag avant de chiffrer, puis l'ajouter à la fin du même
buffer. Le code proposé dans la discussion construit `[tag][payload]` dans
un second buffer ; cela copie tout le payload et diffère du suffixe retenu.

Avec compression, conserver un scratch réutilisable. La compression compare
sa sortie aux octets bruts restés intacts, puis échange les buffers si elle
est rentable. La décompression écrit dans un autre buffer avant échange.
L'API est destructive, mais ne promet pas que la compression soit réellement
effectuée dans la même zone mémoire. Les API block LZ4 prennent une source et
une destination séparées. Voir l'[API du compresseur](https://docs.rs/lz4_flex/latest/lz4_flex/block/fn.compress_into_with_table.html).

Sur erreur, aucun contenu du buffer n'est livrable. Le codec vide le buffer
sur échec sans prétendre effacer toutes les copies physiques en mémoire.
Les trames sont livrées uniquement après authentification et décompression
complètes lorsque ces transformations sont activées.

Le choix `Vec<u8>` minimise les dépendances. Une variante fondée exclusivement
sur `BytesMut` reste à discuter si c'est le type déjà utilisé partout par
l'intégrateur. Il n'y aura pas deux familles d'API uniquement pour ce choix.

## 8. Écriture, finalisation et adaptation CARBON

```rust,ignore
// Code de l'intégrateur, pas une API async fournie par CRAFT.
for (index, mut buffer) in frames {
    let sizes = encoder.encode_frame(index, &mut buffer)?;
    destination.write_all(&buffer).await?;
    metadata.push(sizes)?;
}
destination.flush().await?;
// Finaliser le backend, puis publier données et métadonnées ensemble.
```

Les trames arrivent déjà délimitées. L'appelant choisit leur provenance.

En mode fixe, une trame courte est nécessairement la dernière. Une nouvelle
trame après celle-ci est refusée. En variable, toutes les longueurs entre un
octet et le maximum configuré sont acceptées.

La boucle illustre le partage des responsabilités. Un adaptateur de production
doit également gérer la publication des métadonnées en cas d'erreur et son
contrat d'annulation. Un flush ne garantit ni durabilité disque ni succès
distant ; l'intégrateur finalise son backend.

Pour `ReadFile<T>`, l'adaptateur capture la plage logique dans son descripteur.
Il peut préparer la query et le nombre de trames avant `open()` sans effectuer
d'I/O. Son `open()` sans argument ouvre ensuite uniquement la plage physique.
Son `Stream` accumule les octets d'une trame, appelle le décodeur synchrone et
retourne `Ready(Some(Ok(frame)))`. Le type `T` peut contenir le buffer possédé
et la portion sélectionnée, pour éviter une copie de découpage. Les lectures
partielles et le buffer restent dans le stream entre deux polls. Son compte
correspond aux trames touchées par la sélection ; il vérifie la conversion
vers le `u32` de CARBON. Une vue vide est omise, puisque ce contrat exige un
compte non nul. Le plan de plage permet de connaître ce compte sans allouer
ni collecter l'itérateur.

L'idée de faire fournir les buffers de lecture par CARBON reste une évolution
possible de CARBON. Le trait Stream actuel rend un élément possédé et ne
prend pas de destination. Une lecture dans un buffer prêté demanderait un
contrat distinct, par exemple :

```rust,ignore
fn poll_read_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    dst: &mut T,
) -> Poll<Result<bool, Self::Error>>; // true : trame complète ; false : EOF.
```

Pendant `Pending`, CARBON conserve le même buffer logique et son contenu.
Le reader conserve sa progression sans retenir l'emprunt ni dépendre d'une
adresse stable. Une erreur invalide la sortie ; seule une trame entièrement
lue et vérifiée peut être livrée. Recommencer à écrire le buffer d'une trame
non livrée ne suffit pas à définir un retry : il faut aussi repositionner ou
rouvrir la source et ne pas réémettre les trames déjà livrées.

CARBON traite un `T` opaque : l'appelant doit donc fournir les buffers ou leur
fabrique. Et si CARBON rend ensuite `T` par valeur au consommateur, il perd
son allocation. Pour la réutiliser, le consommateur doit la rendre, ou traiter
une trame empruntée. Le prêt à la lecture seul ne crée pas un recyclage complet.
Ces choix ne modifient pas l'API synchrone de CRAFT et ne sont pas implémentés
dans cette étape.

Pour `FrameWriter<T>`, le `poll_write(&T)` existant emprunte une trame que CARBON
doit pouvoir rejouer. L'adaptateur prépare donc un buffer encodé dont il est
propriétaire. Sans compression, préserver la trame brute impose une copie
avant chiffrement ; avec compression rentable, sa sortie peut directement
servir de buffer au chiffrement.

Si l'écriture retourne `Pending`, l'adaptateur conserve ce buffer et son offset
d'écriture. Le poll suivant continue les I/O sans rechiffrer. Il n'enregistre
la trame qu'une fois, avant de retourner `Ready(Ok(()))`. Aucun emprunt fourni
par CARBON n'est conservé après le retour du poll.

Décision retenue pour l'écriture CARBON : garder `poll_write(&T)` avec ou sans
retry, et conserver l'entrée brute intacte. Aucun second chemin `&mut T` dans
le scheduler. L'adaptateur utilise un buffer de travail mutable et réutilisable.
Un `T::clone()` générique ne garantit pas une copie mutable indépendante des
octets, notamment avec un type partagé ; cette propriété doit être assurée
par le choix du buffer et l'adaptateur.

## 9. Composition et hot path

L'API peut utiliser `Encoder<C, E>` et `Decoder<C, E>`, avec un composant de
compression `C` et un composant de chiffrement `E`. Les variantes identité
suppriment leurs traitements par static dispatch. Le découpage et l'index
restent séparés du codec.

Deux petits contrats internes suffisent : compresser/décompresser vers un buffer,
puis chiffrer/déchiffrer en place. Ils restent privés au début, afin de ne pas
figer une API de plugins avant d'avoir deux implémentations utiles.
Pas de liste dynamique de stages ni d'empilement de wrappers qui possèdent
chacun leurs buffers. L'ordre compression puis chiffrement fait partie du format.

Objectifs à mesurer :

- aucun lock, channel ou tâche interne ;
- aucun `Box<dyn ...>` imposé sur le trajet d'une trame ;
- contexte crypto initialisé une fois par reader ou writer ;
- buffers et contexte de compression réutilisés ;
- aucune allocation par trame après chauffe pour le cœur à buffers réutilisables ;
- lecture sans compression avec déchiffrement en place ;
- erreurs froides et validation du descripteur avant les I/O.

Les contrôles sur les octets reçus, les tailles de décompression et les tags
restent obligatoires pendant le traitement. Les frontières de largeur d'index
et de codec sont sélectionnées à la construction autant que possible.

La mémoire de travail est O(taille maximale de trame), plus le contexte du codec.
L'index variable est O(nombre de trames) et doit être comptabilisé séparément.
Le scratch de compression peut dépasser la taille brute pendant l'essai ;
seul le payload conservé est garanti inférieur ou égal à celle-ci.

La granularité publique de calcul est une trame complète. Introduire un
`Pending` entre compression et chiffrement ne suspend pas une compression
déjà en cours et ne déplace pas le calcul vers un autre thread. Il faudrait
mémoriser une étape et programmer un réveil pour céder volontairement.
Cela permet une coopération entre étapes, sans garantie de préemption au
milieu d'un appel au codec. Voir le [contrat de Future::poll](https://doc.rust-lang.org/std/future/trait.Future.html).

Recommandation : terminer les transformations d'une trame dans le même appel.
Cela évite un passage supplémentaire par l'exécuteur et donne l'occasion de
réutiliser immédiatement les données en cache. L'effet réel sur le cache
dépend du matériel, de la taille et des autres tâches ; ce n'est pas garanti.

64 KiB constitue une première taille de benchmark, pas une garantie de durée
acceptable sur tout processeur et avec tout codec. L'intégrateur mesure le
temps par trame et le nombre de trames traitées par poll. Il peut traiter
directement ou déplacer un buffer possédé vers un worker externe. Une tâche
async ordinaire n'est pas, à elle seule, un offload adapté aux gros calculs.
Les workers CPU doivent avoir une concurrence bornée. Voir les
[recommandations de Tokio](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

CRAFT n'expose ni exécuteur configurable ni protocole de micro-étapes. Les
applications sync et async appellent exactement les mêmes fonctions.

## 10. Bibliothèques candidates

La sélection vise d'abord un petit nombre de dépendances directes. Le meilleur
débit dépendra du processeur, des tailles et des données ; la documentation
ne remplace pas une comparaison sur le matériel cible.

| Besoin | Proposition | Motif |
| --- | --- | --- |
| Chiffrement initial | `aes-gcm`, RustCrypto | AES-256-GCM, traitement en place, tag séparé utilisable en suffixe |
| Première compression | `lz4_flex`, API block | Rust, buffers fournis par l'appelant, contexte réutilisable |
| Compression alternative | `zstd`, API bulk | Candidat pour comparer le compromis taille/temps, contexte réutilisable |
| I/O et traits CARBON | Dépendances de l'intégrateur | Aucune dépendance async dans CRAFT |

`aes-gcm` documente ses opérations en place et les conditions matérielles de
ses implémentations en temps constant. On réutilise ces opérations sans
réimplémenter AES ou GCM. Voir la [documentation RustCrypto](https://docs.rs/aes-gcm/0.11.1/aes_gcm/).

Pour LZ4, utiliser les blocs indépendants sans préfixe de taille, déjà présent
dans les métadonnées. `compress_into_with_table` permet de réutiliser la table,
et `decompress_into` travaille vers une destination fournie. La capacité du
scratch suit `get_maximum_output_size`. Voir l'[API block](https://docs.rs/lz4_flex/0.14.0/lz4_flex/block/index.html).
Conserver les features de sécurité ; désactiver le format frame inutile évite
sa dépendance de checksum. Voir les [features publiées](https://docs.rs/crate/lz4_flex/0.14.0/source/Cargo.toml).

Zstd reste un candidat ultérieur. Son `bulk::Compressor` réutilise le contexte
entre blocs indépendants ; son ajout doit être justifié par les mesures.
Voir l'[API bulk](https://docs.rs/zstd/latest/zstd/bulk/struct.Compressor.html).

ChaCha20-Poly1305 est une alternative à comparer si les machines cibles
défavorisent AES-GCM. RustCrypto propose aussi le traitement en place.
Il n'est pas nécessaire de l'embarquer dès la V1. Voir la
[documentation du codec](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/).

Le choix provisoire est donc AES-GCM, puis LZ4. Pas de Gzip, de dictionnaire
inter-trames ni de négociation de codecs dans la première version.

Le manifeste actuel annonce Rust 1.85. Les manifestes consultés annoncent 1.85
pour [`aes-gcm` 0.11.1](https://docs.rs/crate/aes-gcm/0.11.1/source/Cargo.toml)
et 1.81 pour [`lz4_flex` 0.14.0](https://docs.rs/crate/lz4_flex/0.14.0/source/Cargo.toml).
Cela ne valide pas encore le graphe transitif ni sa compilation.

Les features envisagées sont `aes-gcm` et `lz4`. Le profil identité reste
utilisable sans elles ; le profil usuel active AES-GCM. Le choix entre Tokio,
futures-io ou des I/O synchrones appartient exclusivement à l'intégrateur.

## 11. Organisation du dépôt

Conserver l'organisation plate de CARBON et ajouter les modules quand le lot
correspondant existe :

```text
src/
    lib.rs            exports et documentation
    config.rs         découpage et limites
    metadata.rs       descripteur, validation et index
    codec.rs          encodage et décodage synchrones
    compression.rs    identité puis LZ4
    crypto.rs         identité puis AES-GCM
    error.rs          erreurs typées
tests/
    format.rs
    roundtrip.rs
    corruption.rs
benches/
    frames.rs
docs/
    specification.md
    benchmarks.md
examples/
    memory/
    carbon/           intégration extérieure, dépendances propres à l'exemple
```

Pas de module de scheduler, budget partagé, I/O ou moteur de pipeline.
Les exemples peuvent montrer plusieurs intégrations ; la bibliothèque garde
une seule API de transformation.

À ce stade, le dépôt contient un manifeste, un README minimal, une licence et
un `lib.rs` qui inclut le README. Les dépendances async copiées de CARBON ne
constituent pas encore un choix d'architecture. La proposition synchrone
conduit à les retirer lors de l'implémentation, sauf besoins des exemples.

## 12. Plan de développement

| Lot | Livrable | Critère de sortie |
| --- | --- | --- |
| 0. Contrats | Fixer format, limites et API synchrone unique | Décisions ci-dessous résolues, exemples d'usage relus |
| 1. Framing | Identité fixe/variable, métadonnées compactes et plages logiques sans cache d'offsets | Roundtrip, plages dans une trame et sur plusieurs trames, cas vides et dernière trame |
| 2. AES-GCM | Clé par objet, indice explicite et suffixe tag | Vecteurs connus, corruption, mauvaise clé/index, troncature, limites |
| 3. Compression | LZ4, fallback brut, scratch réutilisable | Mélanges compressé/brut, taille exacte, entrée invalide, mémoire bornée |
| 4. Intégration | Exemple CARBON extérieur à l'API du crate | I/O partielles, Pending sans réencodage, annulation, retries du transport |
| 5. Mesures | Benchmarks CPU, buffers et positionnement | Coûts documentés, choix de buffers confirmé, six familles couvertes |

Le lot 1 utilise l'identité pour isoler le framing ; le premier profil chiffré
utilisable reste le fixe sans compression. Le lot compression arrive après
stabilisation des contrats et conserve les mêmes méthodes publiques.

Les vérifications devront couvrir notamment les seuils 65 535/65 536/65 537 et
`2^32` sans allouer des trames gigantesques, les overflows de sommes, les index
incohérents, les payloads invalides et les échecs après I/O partielle.
Les tests cryptographiques ne se limitent pas à un aller-retour entre deux
fonctions qui pourraient partager la même erreur de format.

Pour les benchmarks, comparer des tailles 4, 16, 64, 256 KiB et 1 MiB sur données
compressibles, incompressibles et mixtes. Mesurer séparément débit, latence
par trame, allocations après chauffe, copies, taille produite, mémoire de
l'index et préparation/ouverture. Comparer chaque profil aux appels directs
aux codecs et publier matériel, versions et features. Les dépendances de
benchmark restent des dev-dependencies. Aucun objectif chiffré n'est promis ici.

## 13. Décisions à confirmer

La proposition est utilisable comme base de discussion avec ces choix :

1. `Metadata::range(start, end)` calcule une plage physique de trames entières
   et un itérateur avec les portions logiques à restituer. Le codec ignore
   ces portions ; l'appelant les sélectionne après décodage.
2. Métadonnées fiables, sans format de conteneur autonome dans le flux.
3. Trames non vides, maximum explicite, tailles encodées en `longueur - 1`.
4. Aucun cache d'offsets cumulés, état de curseur constant en plus de l'index.
   La préparation d'une query bornée parcourt les métadonnées jusqu'à la
   dernière trame nécessaire ; la progression suivante est O(1) par trame.
5. Clé AES indépendante par objet immuable, indice explicite, aucune modification
   du payload sous un couple clé/indice déjà utilisé.
6. Recommandation d'une API uniquement synchrone et destructive sur buffers,
   avec scratch de compression réutilisé. CARBON garde `poll_write(&T)` ;
   le prêt et le recyclage des buffers de lecture sont des sujets séparés.
7. AES-GCM puis LZ4. Type de buffer `Vec<u8>` ou `BytesMut` à confirmer selon
   l'intégration ; pas deux variantes publiques pour couvrir les deux.

Les limites de taille d'objet et d'usage de clé demandent encore les volumes
visés. Elles doivent être fixées avant de considérer le profil crypto stable.
Le choix minimal d'AAD vide et les identifiants de version/codec seront figés
dans le lot 0, avec des exemples exacts d'octets.

Cette étape a produit uniquement la présente spécification. Aucun code Rust,
manifeste ou fichier de CARBON n'a été modifié ; aucun test, build ou benchmark
n'a été exécuté.

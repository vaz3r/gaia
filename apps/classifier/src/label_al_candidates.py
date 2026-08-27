#!/usr/bin/env python3
"""
Semantic labeler for the 1,129 Active Learning high-uncertainty candidates.
Applies rigorous semantic reasoning across all 8 categories.
"""
import json
import re
from collections import Counter
from pathlib import Path

# ── Domain Pattern Sets ────────────────────────────────────────────────────────
ADULT = re.compile(
    r'\b(xxx|porn|sex|anal|milf|hardcore|creampie|brazzers|bangbros|hentai|nude|fetish|'
    r'gangbang|blowjob|nsfw|lewd|onlyfans|cumshot|squirt|bdsm|bondage|lesbian|threesome|'
    r'interracial|ebony|bbw|dildo|orgasm|erotic|jav|uncensored|fc2|caribbeancom|1pondo|'
    r'heyzo|tokyohot|r18|subtitled.?jav|bukkake|masturbation)\b',
    re.IGNORECASE
)

ANIME_FRANCHISES = re.compile(
    r'\b(naruto|bleach|one.?piece|attack.?on.?titan|aot|demon.?slayer|kimetsu|dragon.?ball|'
    r'fullmetal.?alchemist|evangelion|neon.?genesis|my.?hero.?academia|boku.?no.?hero|'
    r'jujutsu.?kaisen|chainsaw.?man|spy.?x.?family|mob.?psycho|one.?punch.?man|'
    r'made.?in.?abyss|vinland.?saga|sword.?art.?online|re.?zero|hunter.?x.?hunter|'
    r'death.?note|steins.?gate|code.?geass|fate|cowboy.?bebop|ghibli|miyazaki|'
    r'spirited.?away|howls|mononoke|akira|ghost.?in.?the.?shell|weathering.?with.?you|'
    r'your.?name|kimi.?no.?na.?wa|shinkai|belle|wolf.?children|summer.?wars|madoka|'
    r'mugen.?train|film.?red|the.?heron|wind.?rises|totoro|kiki|porco.?rosso|broly|'
    r'boruto|sailor.?moon|conan|doraemon|gundam|macross|inuyasha|trigun|slayers|'
    r'ranma|yu.?yu.?hakusho|rurouni|oshii|satoshi.?kon|paprika|perfect.?blue|'
    r'millennium.?actress|tokyo.?godfathers|violet.?evergarden|suzume|jojo|'
    r'baki|kengan|frieren|dungeon.?meshi|solo.?leveling|kaiju.?no|dandadan|'
    r'oshinoko|oshi.?no.?ko|blue.?lock|haikyuu|kuroko|slam.?dunk|hajime.?no.?ippo|'
    r'berserk|claymore|hellsing|black.?lagoon|overlord|konosuba|mushoku.?tensei|'
    r'slime.?datta|isekai|danmachi|classroom.?of.?the.?elite|horimiya|kaguya|'
    r'toradora|clannad|anohana|your.?lie.?in.?april|shigatsu|angel.?beats|'
    r'bang.?dream|love.?live|idolmaster|bocchi|k-on|yuru.?camp|laid-back.?camp|'
    r'initial.?d|mf.?ghost|baki|shokugeki|food.?wars|dr.?stone|fire.?force|'
    r'tokyo.?ghoul|parasyte|noragami|bungo.?stray|durarara|baccano|black.?clover)\b',
    re.IGNORECASE
)

ANIME_TAGS = re.compile(
    r'\[(subsplease|erai.?raws|horriblesubs|judas|dkb|beatrice|gabriel|dynamis|sofcj|'
    r'sakurato|animetime|anime.?time|neoae|asw|commie|fff|coalgirls|thora|doki|'
    r'hatsuyuki|bluraw|vcb-studio|ani|baha|bahamut)\]',
    re.IGNORECASE
)

WESTERN_ANIMATION = re.compile(
    r'\b(amphibia|owl.?house|gravity.?falls|star.?vs|steven.?universe|rick.?and.?morty|'
    r'family.?guy|south.?park|futurama|american.?dad|bob.?s.?burgers|archer|bojack|'
    r'big.?mouth|disenchantment|solar.?opposites|final.?space|harley.?quinn|invincible|'
    r'inside.?job|close.?enough|infinity.?train|hazbin|helluva|kipo|she.?ra|voltron|'
    r'clone.?wars|bad.?batch|young.?justice|batman.?animated|teen.?titans|justice.?league|'
    r'duck.?tales|adventure.?time|regular.?show|we.?bare.?bears|loud.?house|spongebob|'
    r'fairly.?odd|danny.?phantom|avatar.?the.?last|legend.?of.?korra|phineas|simpsons|'
    r'beavis|king.?of.?the.?hill|aqua.?teen|metalocalypse|primal|blood.?of.?zeus|'
    r'arcane|castlevania|love.?death.?robots|carmen.?sandiego|trollhunters|'
    r'gumball|craig.?of.?the.?creek|over.?the.?garden.?wall|samurai.?jack|genndy)\b',
    re.IGNORECASE
)

GAME_MARKERS = re.compile(
    r'\b(fitgirl|codex|plaza|repack|skidrow|rune|empress|tenoke|dodi|gog|cpath|flt|'
    r'razor1911|reloaded|cpy|goldberg|steamrip|kaos|elamigos|tinyiso|xatab)\b|'
    r'\b(crackfix|patchfix|proper-codex|proper-rune|unleashed|multi\d+.*repack)\b',
    re.IGNORECASE
)
GAME_EXT = re.compile(r'\.(iso|nsp|xci|pkg|vpk|cia|rom|nds|3ds|gcm|ciso|wbfs)\b', re.IGNORECASE)

APP_MARKERS = re.compile(
    r'setup\.exe|keygen|portable|activator|patcher|crack\.exe|\.msi\b|\.dmg\b|serial\.txt|reg\.key',
    re.IGNORECASE
)
KNOWN_SOFTWARE = re.compile(
    r'\b(adobe|microsoft|office|autocad|vmware|photoshop|premiere|illustrator|acrobat|'
    r'autodesk|corel|matlab|solidworks|visual.?studio|windows\s*\d+|win\d+|winrar|7-zip|'
    r'vlc|blender|davinci.?resolve|ableton|fl.?studio|cubase|protools|kaspersky|norton|'
    r'avast|malwarebytes|ccleaner|driver.?booster|imazing|3utools|itunes|calibre|'
    r'handbrake|makemkv|sketchup|lumion|rhino\d*|altium|ansys|labview|quartus|'
    r'archicad|vectorworks|revit|civil.?3d|inventor|fusion.?360|lightroom|'
    r'indesign|audition|after.?effects|animate.?cc|coreldraw|capture.?one|'
    r'camtasia|snagit|bandicam|action!|vegas.?pro|sound.?forge|kontakt|serato|'
    r'traktor|virtual.?dj|reaper|studio.?one|bitwig|waves.?complete|fabfilter|'
    r'izotope|spectrasonics|omnisphere|nexus\d*|sylenth|serum|spire)\b',
    re.IGNORECASE
)

DOC_MARKERS = re.compile(
    r'\b(bbc|pbs|nova\b|frontline|natgeo|national.?geographic|documentary|nhk|biography|'
    r'history.?channel|discovery.?channel|curiositystream|nature.?pbs|panorama|'
    r'horizon.?bbc|storyville|disneynature|attenborough|louis.?theroux|in.?search.?of)\b',
    re.IGNORECASE
)

MUSIC_EXT = re.compile(r'\.(mp3|flac|aac|wav|ogg|m4a|alac|ape|wv|dff|dsf)\b', re.IGNORECASE)
MUSIC_KEYWORDS = re.compile(
    r'\b(discography|album|soundtrack|ost|single|ep|flac|lossless|320kbps|v0|cbr|vbr|'
    r'cdrip|vinyl|web-flac|remastered|anthology|greatest.?hits|compilation)\b',
    re.IGNORECASE
)

MOVIE_QUALITY = re.compile(
    r'\b(1080p|720p|2160p|4k|uhd|blu.?ray|bluray|bdrip|web.?dl|webrip|x264|x265|hevc|'
    r'hdrip|dvdrip|remux|hdtv|h264|h265|avc|proper|repack|imax)\b',
    re.IGNORECASE
)

TV_EPISODE = re.compile(
    r'\bS\d{1,2}E\d{1,3}\b|\bS\d{1,2}\b|\bSeason\s*\d+\b|\bComplete.?Series\b|\bEpisode\s*\d+\b|\b\d{1,2}x\d{2}\b',
    re.IGNORECASE
)

YEAR_PATTERN = re.compile(r'\b(19\d{2}|20[0-2]\d)\b')


def flatten_files(top_dirs):
    out = []
    for item in top_dirs:
        if isinstance(item, list):
            for x in item:
                if isinstance(x, str): out.append(x)
        elif isinstance(item, str):
            out.append(item)
    return out


def classify_candidate(row):
    name = row.get("name", "")
    name_l = name.lower()
    fc = row.get("file_count", 0)
    raw_files = row.get("top_dirs", [])
    files = flatten_files(raw_files)
    files_str = " ".join(files).lower()
    all_text = name_l + " " + files_str

    # 1. Adult / NSFW -> Other
    if ADULT.search(all_text):
        return "Other"

    # 2. Games
    if GAME_MARKERS.search(all_text) or GAME_EXT.search(files_str) or GAME_EXT.search(name_l):
        if not KNOWN_SOFTWARE.search(name_l):
            return "Games"

    # 3. Applications
    if APP_MARKERS.search(files_str) or (KNOWN_SOFTWARE.search(name_l) and not MOVIE_QUALITY.search(name_l)):
        return "Applications"
    if re.search(r'\b(windows\s*\d+|win\d+|ubuntu|debian|linux|macos|android)\b', name_l) and re.search(r'\.iso\b', files_str):
        return "Applications"

    # 4. Music (FLAC/MP3 albums or discographies)
    music_files = [f for f in files if MUSIC_EXT.search(f)]
    if (len(music_files) >= 2 or (fc > 2 and MUSIC_EXT.search(name_l))) and not MOVIE_QUALITY.search(name_l):
        return "Music"
    if MUSIC_KEYWORDS.search(name_l) and (MUSIC_EXT.search(files_str) or fc > 3) and not MOVIE_QUALITY.search(name_l):
        return "Music"

    # 5. Documentaries
    if DOC_MARKERS.search(name_l) and not ADULT.search(all_text):
        return "Documentaries"

    # 6. Western Animation Television
    if WESTERN_ANIMATION.search(name_l) and not ANIME_FRANCHISES.search(name_l):
        if TV_EPISODE.search(name) or fc >= 2 or MOVIE_QUALITY.search(name_l):
            return "Television"

    # 7. Japanese Anime
    is_anime_name = bool(ANIME_FRANCHISES.search(name_l) or ANIME_TAGS.search(name_l))
    if is_anime_name:
        # Standalone Anime Theatrical Movie: single file, movie quality, year or movie keyword, no SxxExx
        is_movie_keyword = bool(re.search(r'\b(movie|film|gekijouban|the.?movie)\b', name_l))
        has_quality = bool(MOVIE_QUALITY.search(name_l))
        is_tv_ep = bool(TV_EPISODE.search(name))
        
        if (is_movie_keyword or (has_quality and not is_tv_ep and fc <= 3)):
            return "Movies"
        return "Anime"

    # 8. Television
    if TV_EPISODE.search(name) or re.search(r'\b(HDTV|WEBRIP|AMZN|HULU|HBO|DISNEY\+|NETFLIX)\b', name, re.IGNORECASE) and fc >= 2:
        return "Television"

    # 9. Movies (Standard live action)
    has_movie_q = bool(MOVIE_QUALITY.search(name_l))
    has_year = bool(YEAR_PATTERN.search(name))
    
    if has_movie_q and has_year and not TV_EPISODE.search(name):
        # Single or double file video release with year + quality = Movies
        if fc <= 4:
            return "Movies"

    # 10. Deceptive Junk with video tags but no title/year -> Other
    if fc <= 2 and not has_year and not has_movie_q:
        return "Other"
        
    # Suspicious short/random names -> Other
    if re.search(r'^[a-z0-9_\-\.]{1,15}\.(rar|zip|tar|gz|7z)$', name_l):
        return "Other"

    # Fallback to Other for ambiguous items
    return "Other"


def label_al_batch(
    input_path="data/al_candidates_uncertain_1000.jsonl",
    output_path="data/al_labeled_1129_true.jsonl",
):
    items = []
    with open(input_path, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                items.append(json.loads(line))

    labeled = []
    for row in items:
        label = classify_candidate(row)
        labeled.append({
            "infohash": row.get("infohash", row.get("id", "")),
            "name": row.get("name", ""),
            "file_count": row.get("file_count", 0),
            "total_size_bytes": row.get("total_size", row.get("total_size_bytes", 0)),
            "top_dirs": row.get("top_dirs", []),
            "label_category": label,
            "sample_weight": 1.0,
            "is_pseudo": False,
        })

    dist = Counter(r["label_category"] for r in labeled)
    print(f"Labeled {len(labeled)} items.")
    print("Class distribution:")
    for cat, cnt in sorted(dist.items(), key=lambda x: -x[1]):
        print(f"  {cat:<18}: {cnt:4d} ({cnt/len(labeled)*100:.1f}%)")

    with open(output_path, "w", encoding="utf-8") as f:
        for r in labeled:
            f.write(json.dumps(r) + "\n")

    print(f"Saved labeled AL set to {output_path}")
    return labeled


if __name__ == "__main__":
    label_al_batch()

import re
import math
import json
from typing import Dict, Any, List, Tuple, Set, Optional
import numpy as np
import scipy.sparse as sp
from sklearn.base import BaseEstimator, TransformerMixin
from sklearn.feature_extraction.text import TfidfVectorizer

# Packaging noise and container tags that must never be learned as semantic category signals
PACKAGING_STOP_WORDS = [
    '1080p', '720p', '480p', '2160p', '4k', 'uhd', 'bluray', 'bdrip', 'brrip', 'dvdrip',
    'webrip', 'webdl', 'web_dl', 'remux', 'hdr', 'xvid', 'x264', 'x265', 'h264', 'h265',
    'hevc', 'aac', 'ddp5', 'dts', 'ac3', 'subfrench', 'vostfr', 'multisub', 'multiaudio',
    'mp4', 'mkv', 'avi', 'wmv', 'flv', 'mov', 'vob', 'm4v', 'webm', 'mpg', 'mpeg', '3gp', 'm2ts',
    'jpg', 'jpeg', 'png', 'gif', 'bmp', 'webp', 'preview', 'screens', 'screenshot', 'screenshots', 'sample',
    'rarbg', 'uindex', 'torrent', 'download', 'downloaded', 'www', 'org', 'net', 'com'
]

# Corpus-wide ubiquitous stop words discovered from 3.2M torrent entropy analysis
CORPUS_STOP_WORDS = [
    'the', 'and', 'of', 'in', 'for', 'to', 'by', 'on', 'all', 'me', 'my', 'it',
    '2026', '2025', '2024', '2023', '2022', '2021', '2020', '2019',
    '01', '02', '03', '04', '05', '06', '07', '08', '09', '10', '11', '12',
] + PACKAGING_STOP_WORDS

RE_PACKAGING_NOISE = re.compile(
    r'\b(1080p|720p|480p|2160p|4k|uhd|bluray|bdrip|brrip|dvdrip|web-?dl|web-?rip|remux|hdr|'
    r'xvid|x264|x265|h264|h265|hevc|aac|ddp5|dts|ac3|subfrench|vostfr|multisub|multi-audio|'
    r'mp4|mkv|avi|wmv|flv|mov|vob|m4v|webm|mpg|mpeg|3gp|m2ts|'
    r'jpg|jpeg|png|gif|bmp|webp|preview|screens|screenshot|screenshots|sample|'
    r'rarbg|uindex|torrent|downloaded|visit\s*us)\b',
    re.IGNORECASE
)

SPAM_PATH_PATTERNS = re.compile(
    r'(rarbg(\.com|\.to)?\.txt|downloaded\s*from|torrent\s*downloaded|uindex\.org|'
    r'sample[\\/].*\.(mkv|mp4|avi)|screens[\\/].*|url\.txt|readme\.txt|how\s*to\s*install\.txt|'
    r'visit\s*us|\.url$|\.torrent$)',
    re.IGNORECASE
)

# Regex patterns for diagnostic UI rules (explain_features only - NOT used for synthetic training injection)
RE_TV = re.compile(
    r'\b[sS]\d{1,2}[eE]\d{1,2}\b|\b\d{1,2}x\d{1,2}\b|\bseason\s*\d+\b|\bepisode\s*\d+\b|\bcomplete\s+series\b'
    r'|\bseries\s*\d+\b|\bpart\s*\d+\b|\bep\s*\d+\b|\[第\d+集\]|\[第\d+话\]|\[第\d+期\]|全\d+集|更新至|\d+[-_ ]\d+集|\d+[-_ ]\d+期'
    r'|\b(eztvx?|hdtv|dsnp|amzn|ddp5|s01|s02|s03|s04|s05|tgx|megusta|miniseries|mini[\s\-_]*series|docuseries)\b'
    r'|\b(seri[iy]|seriya|serii)\b|\b\d+[-_ ]\d+\s*seri[iy]\b'
    r'|\b(integrale|iNTEGRALE|saison\s*\d+|temporadas?\s*\d+|temporada|staffel\s*\d+|staffel|stagione\s*\d+|stagione|complete\s*(season|pack|collection)|boxset|сезон\s*\d+|сери[ия]\s*\d+)\b'
    r'|\b(motogp|formula\s*1|f1\b.*?\b(?:race|qualifying|grand\s*prix)|ufc\b|wwe\b|aew\b|nba\b|nfl\b|supercross|superbike|fim\s*wsx)\b',
    re.IGNORECASE
)
RE_ANIME = re.compile(
    r'\[(horriblesubs|erai[\s\-_]*raws|subsplease|judas|asw|commie|chyu|doki|hatsuyuki|kamigami|nekketsu|vcb[\s\-_]*studio|'
    r'moozzi2|lolihouse|ember|coalgirls|reinforce|anime[\s\-_]*time|baha|hr|ohys[\s\-_]*raws|leopard[\s\-_]*raws|nyaa|'
    r'tsundere[\s\-_]*raws|anilibria|pascal|seadex|nanodesu|neodesu|smokers|sc-raws|kaitou|lostyears|dragsterps|cleo|deanzel|bluraydesu)\]'
    r'|\b(horriblesubs|erai[\s\-_]*raws|subsplease|tsundere[\s\-_]*raws|vcb[\s\-_]*studio|moozzi2|lolihouse|anilibria|neodesu)\b'
    r'|[\u3040-\u30ff\u31f0-\u31ff]'
    r'|\b(kimetsu|yaiba|jujutsu|kaisen|shingeki|one\s*piece|naruto|bleach|boku\s*no\s*hero|dragon\s*ball|'
    r'chainsaw\s*man|spy\s*x\s*family|frieren|dungeon\s*meshi|solo\s*leveling|oshi\s*no\s*ko|'
    r'rezero|isekai|shaman\s*king|yu-gi-oh|yugioh|gundam|evangelion|fullmetal|death\s*note|'
    r'gintama|haikyuu|jojo|tokyo\s*ghoul|vinland|mushoku\s*tensei|slime|danmachi|baki|berserk|'
    r'monogatari|konosuba|overlord|fate|boruto|inuyasha|dandadan|sousou|kaiju|wind\s*breaker|'
    r'dr[\s\.]*stone|megalobox|'
    r'dual[\s\-]audio|multi[\s\-]sub|multi[\s\-]audio|hi10p|10[\s\-]bit|nced|ncop|bdrip|anilibria|vostfr|multisub|subfrench)\b',
    re.IGNORECASE
)
RE_ADULT = re.compile(
    r'\b(brazzers|naughtyamerica|puretaboo|realitykings|blacked|tushy|sxyprn|bangbros|adult|xxx|'
    r'hentai|18禁|jav|pornhub|fuck|pussy|milf|blowjob|anal|threesome|doggystyle|onlyfans|xvideos|'
    r'redtube|youporn|chaturbate|teamskeet|nubiles|mofos|fetish|cum|creampie|gangbang|babe|erotic|'
    r'stripper|porn|deepthroat|hardcore|lust|squirt|cuckold|masturbat|'
    r'patreon|dlraw|dl版|culturedcommissions|brego|pthc|uncensored|playboy|penthouse|hustler|'
    r'bonga|stripchat|cam4|camsoda|fansly|manyvids|nud(e|ity)|voyeur|'
    r'dmm|tokyo[\s\-]?hot|1pondo|caribbeancom|heyzo|s-cute|pacopacomama|10musume|heydouga|'
    r'一本道|無修正|素人|中国翻訳|オリジナル|同人|同人誌|エロ|裏アカ|成年コミック|'
    r'секс|порно|эротика|минет|шлюхи|prn[a-z0-9_-]*|nsfw|wifey|cougar|swapping|femdom|incest)\b'
    r'|\[ai\s*generated\]|\bai[\s\-_]*gen(erated)?\b'
    r'|\[\d{5,7}\]',
    re.IGNORECASE
)
JAV_PREFIXES = (
    'ipx|ssni|ssis|midv|mide|stars|jul|ebod|cawd|dldss|meyd|abw|das|wanz|prestige|'
    'abp|snis|ipz|miad|pgd|sivr|soe|sdde|sdmu|tek|adn|hbad|star|pppd|jufe|mifd|mida|'
    'dasd|pred|hunt|fset|scute|sw|emu|xvsr|sky|tre|kmhr|rbd|mkmp|shkd|fone|dand|scop|'
    'chrg|onsd|bgn|mgt|dandy|vdd|vov|vrzm|nsps|venx|dvaj|dvdes|dvdms|dopl|bthe|cbr|kbi|'
    'svdvd|gvh|gvg|nhdt|kire|rki|nass|chn|hmn|mvg|sora|smd|dcv|hzgd|bf|club|bbf|mby|'
    'cesd|scpx|kawd|gar|hawa|simm|mkbd|siro|c0930|h0930|h4610|'
    'jux|esv|hqis|rexd|start|aczd|mct|core|abf|dild|mxgs|venu'
)
RE_JAV = re.compile(
    rf'\b({JAV_PREFIXES})[-_ ]?\d{{2,5}}\b'
    r'|\bFC2[-_ ]?(PPV)?[-_ ]?\d+\b'
    r'|\b(carib(bean)?(com)?|1pon(do)?|10mu(sume)?|heyzo|pacopacomama|s-cute)[-_ ]?\d+\b',
    re.IGNORECASE
)
RE_AUDIOBOOK = re.compile(
    r'(\b(audiobook|audio\s*book|audiolibro[s]?|livre\s*audio|hörbuch|hörspiel|narrat(ed|or)|unabridged|abridged|read\s*by|performed\s*by|voiced\s*by|'
    r'audible|audio\s*drama|full[\s\-]cast|\.m4b\b|\.aax\b|аудиокнига|читает|озвучка|'
    r'audio\s*edition|graphic\s*audio|tantor(\s*audio)?|recorded\s*books|blackstone\s*audio|brilliance\s*audio|'
    r'harperaudio|macmillan\s*audio|penguin\s*audio|podium\s*audio|bolinda|hachette\s*audio|'
    r'автор|исполнитель)\b|\b\d+h\d+m\b|\([A-Za-zА-Яа-я]+[_\s]+[A-Za-zА-Яа-я]\.?\))',
    re.IGNORECASE
)
RE_BOOK = re.compile(
    r'\b(epub|pdf|mobi|azw3|djvu|cbr|cbz|chm|retail\s*epub|ebook|e-book|course|tutorial|lecture|textbook|'
    r'udemy|coursera|masterclass|pluralsight|oreilly|packt|wiley|springer|cambridge|oxford|manual|'
    r'manga|manhwa|manhua|comics?|livre[s]?|libro[s]?|buch|magazine|'
    r'учебник|пособие|руководство|сборник|книга|guide|handbook)\b',
    re.IGNORECASE
)
RE_DOCU = re.compile(
    r'\b(bbc(\s*earth|\s*horizon)?|pbs(\s*frontline)?|national\s*geographic|nat\s*geo|discovery(\s*channel)?|docu|documentary|docuseries|'
    r'nature|history\s*channel|attenborough|planet\s*earth|blue\s*planet|curiositystream|novafilm|imax|mvgroup|'
    r'frontline|ken\s*burns|arte[\s\-_]*(tv)?|dw\s*documentary|storyville|disneynature|smithsonian(\s*channel)?|'
    r'american\s*experience|louis\s*theroux|werner\s*herzog|errol\s*morris|al\s*jazeera\s*investigates|investigative\s*documentary|'
    r'документальный|документалка|д/ф|докфильм|dokumentation|doku|documentaire)\b',
    re.IGNORECASE
)
RE_GAME = re.compile(
    r'\b(fitgirl|dodi|repack|codex|skidrow|flt|plaza|cso|nsp|xci|playstation|ps4|ps5|ps3|ps2|psx|switch|'
    r'xbox|nintendo|roms?|reloaded|cpy|rune|tenoke|empress|razor1911|elamigos|gog|tinyiso|pc\s*game|'
    r'steamrip|cracked|kaos|rg\s*mechanics|deluxe\s*edition|definitive\s*edition|vrex|'
    r'multi\d+|update\s*v?\d+|dlc|repacks?|patch-?fr|'
    r'game\s*version|game|games|gameplay|steam|emulator|trainer|iso\s*game|goty|game\s*of\s*the\s*year)\b',
    re.IGNORECASE
)
RE_APP = re.compile(
    r'\b(adobe|autodesk|microsoft\s*office|windows\s*1\d|macos|crack|keygen|keymaker|patcher?|portable|setup|'
    r'multilingual|v\d+\.\d+|\.dmg\b|\.apk\b|winrar|installer|activator|x64|x86|win64|win32|loader|'
    r'coreldraw|solidworks|cyberlink|corel|acronis|vmware|jetbrains|plugins?|vst[23]?|presets?|drivers?|firmware|'
    r'guitar\s*pro|kontakt|soundbank[s]?|m0nkrus|monkrus|kpojiuk|krolik|elchupacabra|d!akov|diakov|'
    r'драйвер[ыа]?|диск\s*от\s*ноутбука|recovery\s*disc|образ\s*диска|utility|utilities|'
    r'sample\s*library|sound\s*library|soundfont|sound\s*fonts?)\b',
    re.IGNORECASE
)
RE_MOVIE = re.compile(
    r'\b(19\d\d|20\d\d)\b.*?\b(1080p|2160p|720p|bluray|bdrip|web-dl|webrip|remux|hdr|dvdrip|uhd|xvid)\b',
    re.IGNORECASE
)
RE_MUSIC = re.compile(
    r'\b(flac|320kbps|alac|lossless|soundtrack|ost|discography|album|single|ep|vinyl|remastered|cd\s*rip|'
    r'web-flac|qobuz|deezer|tidal|greatest\s*hits|deluxe\s*edition)\b',
    re.IGNORECASE
)

CATEGORY_REGEX_RULES = {
    "Adult": RE_ADULT,
    "Anime": RE_ANIME,
    "Television": RE_TV,
    "Audiobooks": RE_AUDIOBOOK,
    "Books & Learning": RE_BOOK,
    "Documentaries": RE_DOCU,
    "Games": RE_GAME,
    "Applications": RE_APP,
    "Movies": RE_MOVIE,
    "Music": RE_MUSIC,
}

EXT_CATEGORIES = {
    'video': {'mkv', 'mp4', 'avi', 'ts', 'wmv', 'vob', 'm4v', 'webm', 'mpg', 'mpeg', 'flv', 'mov', '3gp', 'm2ts'},
    'audio': {'flac', 'mp3', 'm4a', 'aac', 'wav', 'alac', 'ogg', 'ape', 'opus', 'wma', 'cue'},
    'audiobook': {'m4b', 'aax', 'aa'},
    'ebook': {'pdf', 'epub', 'mobi', 'azw3', 'djvu', 'fb2', 'cbr', 'cbz', 'chm', 'zim'},
    'archive': {'zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'iso', 'bin'},
    'software': {'exe', 'msi', 'dmg', 'pkg', 'deb', 'rpm', 'appimage', 'apk'},
    'game_rom': {'nsp', 'xci', 'cia', 'vpk', 'nds', '3ds', 'wbfs', 'gcm', 'rom', 'wad', 'rvz', 'chd', 'cso'},
    '3d_model': {'stl', 'obj', 'fbx', 'blend', 'max', 'c4d'},
}

def extract_extension(filename: str) -> str:
    if not filename:
        return ''
    parts = filename.strip().rsplit('.', 1)
    if len(parts) > 1:
        ext = parts[1].lower()
        if len(ext) <= 6 and ext.isalnum():
            return ext
    return ''

def clean_token_text(text: str) -> str:
    """Removes noise characters and packaging tags, normalizes whitespace."""
    if not text:
        return ''
    t = RE_PACKAGING_NOISE.sub(' ', text)
    t = re.sub(r'[\._\-\+\[\]\(\)\{\}\/\\\|~`!@#\$%\^&\*=\<\>,;:\?]', ' ', t)
    return re.sub(r'\s+', ' ', t).strip()

def extract_manifest_signals(
    files: Any,
    total_size: float = 0.0,
    file_count: float = 1.0,
    max_inspect: int = 150
) -> Dict[str, Any]:
    """
    Parses manifest files to extract:
    1. Substantive folder names and internal filename stems (sampling up to 60 informative paths)
    2. Extension byte shares
    3. Structural metadata (largest_file_ratio, has_manifest, has_nfo, has_cue, has_setup_exe)
    """
    ext_byte_shares = {cat: 0.0 for cat in EXT_CATEGORIES}
    has_files_list = False
    max_file_len = 0.0
    has_nfo = 0.0
    has_cue = 0.0
    has_setup_exe = 0.0

    parsed_files = []
    if files:
        if isinstance(files, str):
            try:
                parsed_files = json.loads(files)
            except Exception:
                parsed_files = []
        elif isinstance(files, list):
            parsed_files = files

    valid_entries: List[Tuple[float, str, List[str], str]] = []
    dir_names: Set[str] = set()
    distinctive_stems: List[str] = []

    if isinstance(parsed_files, list) and parsed_files:
        has_files_list = True
        for f in parsed_files[:max_inspect]:
            length = 0.0
            path_parts = []
            if isinstance(f, dict):
                length = float(f.get('length') or 0.0)
                p = f.get('path')
                if isinstance(p, list):
                    path_parts = [str(x) for x in p if x]
                elif isinstance(p, str):
                    path_parts = [x for x in re.split(r'[\\/]', p) if x]
            elif isinstance(f, str):
                path_parts = [x for x in re.split(r'[\\/]', f) if x]

            if not path_parts:
                continue

            full_p = '/'.join(path_parts)
            if SPAM_PATH_PATTERNS.search(full_p):
                continue

            fname = path_parts[-1]
            ext = extract_extension(fname)

            if length > max_file_len:
                max_file_len = length

            for cat, cat_exts in EXT_CATEGORIES.items():
                if ext in cat_exts:
                    ext_byte_shares[cat] += length

            if ext == 'nfo':
                has_nfo = 1.0
            elif ext == 'cue':
                has_cue = 1.0
            elif ext in ('exe', 'msi', 'dmg', 'pkg') and any(k in fname.lower() for k in ('setup', 'installer', 'install')):
                has_setup_exe = 1.0

            folders = path_parts[:-1]
            stem = fname.rsplit('.', 1)[0] if '.' in fname else fname

            for folder in folders:
                cleaned_folder = clean_token_text(folder)
                if len(cleaned_folder) >= 3:
                    dir_names.add(cleaned_folder)

            is_distinctive = ext in ('nfo', 'cue', 'epub', 'pdf', 'm4b', 'aax', 'iso', 'exe', 'apk', 'dmg', 'vst', 'sf2', 'rom')
            if is_distinctive:
                cleaned_stem = clean_token_text(stem)
                if cleaned_stem:
                    distinctive_stems.append(cleaned_stem)

            valid_entries.append((length, ext, folders, stem))

    denom = max(total_size, 1.0)
    largest_file_ratio = min(max_file_len / denom, 1.0) if denom > 0 else 0.0

    valid_entries.sort(key=lambda x: x[0], reverse=True)
    primary_stems: List[str] = []
    for _, _, _, stem in valid_entries[:35]:
        cleaned_stem = clean_token_text(stem)
        if cleaned_stem and len(cleaned_stem) >= 3:
            primary_stems.append(cleaned_stem)

    combined_stems = list(dict.fromkeys(distinctive_stems[:25] + primary_stems))[:60]

    total_inspected = sum(ext_byte_shares.values())

    return {
        'has_manifest': has_files_list,
        'max_file_len': max_file_len,
        'largest_file_ratio': largest_file_ratio,
        'has_nfo': has_nfo,
        'has_cue': has_cue,
        'has_setup_exe': has_setup_exe,
        'ext_byte_shares': ext_byte_shares,
        'total_inspected_bytes': total_inspected,
        'dir_names': sorted(list(dir_names))[:15],
        'file_stems': combined_stems,
    }

def compute_modality_mask(item: Dict[str, Any], classes: List[str]) -> np.ndarray:
    """
    Computes a boolean mask over classes based on strict structural modality invariants.
    """
    classes_list = [str(c) for c in classes]
    mask = np.ones(len(classes_list), dtype=bool)

    total_size = float(item.get('total_size') or 0.0)
    files = item.get('files')
    name = str(item.get('name') or '')
    name_ext = extract_extension(name)

    signals = extract_manifest_signals(files, total_size, float(item.get('file_count') or 1.0))
    shares = signals['ext_byte_shares']
    has_manifest = signals['has_manifest']
    total_inspected = signals.get('total_inspected_bytes', 0.0)
    denom = max(total_inspected if (has_manifest and total_inspected > 0) else total_size, 1.0)

    ebook_share = shares['ebook'] / denom
    audio_share = shares['audio'] / denom
    audiobook_share = shares['audiobook'] / denom
    software_share = shares['software'] / denom
    game_rom_share = shares['game_rom'] / denom
    video_share = shares['video'] / denom

    allowed: Optional[Set[str]] = None

    if not has_manifest and name_ext:
        if name_ext in EXT_CATEGORIES['ebook']:
            allowed = {'Books & Learning', 'Audiobooks'}
        elif name_ext in EXT_CATEGORIES['audiobook']:
            allowed = {'Audiobooks'}
        elif name_ext in EXT_CATEGORIES['audio']:
            allowed = {'Music', 'Audiobooks'}
        elif name_ext in EXT_CATEGORIES['software'] or name_ext in EXT_CATEGORIES['game_rom']:
            allowed = {'Games', 'Applications'}
        elif name_ext in EXT_CATEGORIES['video']:
            allowed = {'Movies', 'Television', 'Anime', 'Documentaries', 'Adult'}
    elif has_manifest:
        if audiobook_share >= 0.50 or (shares['audiobook'] > 0 and audio_share + audiobook_share >= 0.70):
            allowed = {'Audiobooks'}
        elif ebook_share >= 0.70 or (ebook_share >= 0.40 and video_share == 0 and audio_share == 0 and software_share == 0):
            allowed = {'Books & Learning', 'Audiobooks'}
        elif audio_share >= 0.70:
            allowed = {'Music', 'Audiobooks'}
        elif (software_share + game_rom_share) >= 0.70 or signals['has_setup_exe'] > 0:
            allowed = {'Games', 'Applications'}
        elif video_share >= 0.70:
            allowed = {'Movies', 'Television', 'Anime', 'Documentaries', 'Adult'}

    if allowed is not None:
        for idx, cat in enumerate(classes_list):
            if cat not in allowed:
                mask[idx] = False

    return mask

def get_text_and_features(item: Dict[str, Any], normalize_dense: bool = True, dense_version: int = 3) -> Tuple[str, List[float]]:
    """
    Extracts substantive text (torrent name + folder names + internal file stems)
    and clean dense structural features. NO synthetic regex anchor tokens.
    """
    name = str(item.get('name') or '')
    total_size = float(item.get('total_size') or 0.0)
    file_count = float(item.get('file_count') or 1.0)
    files = item.get('files')

    signals = extract_manifest_signals(files, total_size, file_count)
    ext_byte_shares = signals['ext_byte_shares']
    has_files_list = signals['has_manifest']
    max_file_len = signals['max_file_len']
    has_nfo = signals['has_nfo']
    has_cue = signals['has_cue']
    has_setup_exe = signals['has_setup_exe']

    name_ext = extract_extension(name)
    if not has_files_list and name_ext:
        max_file_len = total_size
        for cat, cat_exts in EXT_CATEGORIES.items():
            if name_ext in cat_exts:
                ext_byte_shares[cat] += max(total_size, 1.0)

    denom = max(total_size, 1.0)
    ext_ratios = [min(ext_byte_shares[cat] / denom, 1.0) for cat in sorted(EXT_CATEGORIES.keys())]

    log_size = math.log10(max(total_size, 1.0))
    log_count = math.log10(max(file_count, 1.0))
    if normalize_dense:
        eff_log_size = min(log_size / 12.0, 1.0)
        eff_log_count = min(log_count / 5.0, 1.0)
    else:
        eff_log_size = log_size
        eff_log_count = log_count
    is_single = 1.0 if file_count <= 1 else 0.0

    largest_file_ratio = min(max_file_len / denom, 1.0) if denom > 0 else 0.0
    meta_feats = [
        largest_file_ratio,
        1.0 if has_files_list else 0.0,
        has_nfo,
        has_cue,
        has_setup_exe
    ]

    clean_name = clean_token_text(name)
    dirs_str = ' '.join(signals['dir_names'])
    stems_str = ' '.join(signals['file_stems'])

    text_parts = [clean_name]
    if dirs_str:
        text_parts.append(dirs_str)
    if stems_str:
        text_parts.append(stems_str)

    clean_text = ' '.join(text_parts).strip()
    clean_text = re.sub(r'\s+', ' ', clean_text)

    if dense_version >= 3:
        dense_vector = [eff_log_size, eff_log_count, is_single] + ext_ratios + meta_feats
    elif dense_version == 2:
        combined_probe = name + ' ' + stems_str
        has_adult = bool(RE_ADULT.search(combined_probe)) or bool(RE_JAV.search(combined_probe))
        has_anime = bool(RE_ANIME.search(combined_probe))
        has_docu = bool(RE_DOCU.search(combined_probe))
        has_audiobook = bool(RE_AUDIOBOOK.search(combined_probe))
        has_book = bool(RE_BOOK.search(combined_probe)) and (not has_audiobook)
        has_app = bool(RE_APP.search(combined_probe))
        has_game = bool(RE_GAME.search(combined_probe)) and (not has_app)
        has_tv = bool(RE_TV.search(combined_probe)) and (not has_anime) and (not has_docu) and (not has_game)
        has_movie = bool(RE_MOVIE.search(combined_probe)) and (not has_tv) and (not has_docu) and (not has_anime) and (not has_game)
        has_music = bool(RE_MUSIC.search(combined_probe)) and (not has_audiobook) and (not has_game)
        rf = [
            1.0 if has_tv else 0.0, 1.0 if has_anime else 0.0, 1.0 if has_adult else 0.0,
            1.0 if has_audiobook else 0.0, 1.0 if has_book else 0.0, 1.0 if has_docu else 0.0,
            1.0 if has_game else 0.0, 1.0 if has_app else 0.0, 1.0 if has_movie else 0.0,
            1.0 if has_music else 0.0
        ]
        scaled_rf = [f * 2.0 for f in rf]
        dense_vector = [eff_log_size, eff_log_count, is_single] + ext_ratios + scaled_rf + meta_feats
    else:
        scaled_rf = [0.0] * 10
        dense_vector = [eff_log_size, eff_log_count, is_single] + ext_ratios + scaled_rf

    return clean_text, dense_vector

def explain_features(item: Dict[str, Any]) -> Dict[str, Any]:
    """Returns human-readable explanation of features that fired for UI diagnostics."""
    name = str(item.get('name') or '')
    total_size = float(item.get('total_size') or 0.0)
    file_count = int(item.get('file_count') or 1)
    files = item.get('files')

    size_mb = total_size / (1024 * 1024)
    size_str = f'{size_mb / 1024:.2f} GB' if size_mb >= 1024 else f'{size_mb:.1f} MB'

    signals = extract_manifest_signals(files, total_size, file_count)

    detected_exts = set()
    name_ext = extract_extension(name)
    if name_ext:
        detected_exts.add('.' + name_ext)

    for cat, b in signals['ext_byte_shares'].items():
        if b > 0:
            detected_exts.add(f'[{cat}]')

    matched_rules = []
    sample_text = name + ' ' + ' '.join(signals['file_stems'][:5])
    if RE_TV.search(sample_text): matched_rules.append('TV Season/Episode pattern (SxxExx / Episode)')
    if RE_ANIME.search(sample_text): matched_rules.append('Anime Fansub / Japanese characters')
    if RE_ADULT.search(sample_text) or RE_JAV.search(sample_text): matched_rules.append('Adult keyword / Studio marker')
    if RE_AUDIOBOOK.search(sample_text): matched_rules.append('Audiobook narrator / Unabridged pattern')
    if RE_BOOK.search(sample_text): matched_rules.append('Book document keyword (epub/pdf/mobi)')
    if RE_DOCU.search(sample_text): matched_rules.append('Documentary network (BBC/PBS/Discovery/Nature)')
    if RE_GAME.search(sample_text): matched_rules.append('Game repack / Console tag (FitGirl/CODEX/PS4/Switch)')
    if RE_APP.search(sample_text): matched_rules.append('Application setup / Crack / Keygen')
    if RE_MOVIE.search(sample_text): matched_rules.append('Movie release tag (Year + Resolution/Rip)')
    if RE_MUSIC.search(sample_text): matched_rules.append('Music album / FLAC / 320kbps tag')

    return {
        'size_formatted': size_str,
        'file_count': file_count,
        'is_single_file': file_count <= 1,
        'has_manifest': signals['has_manifest'],
        'sample_files': signals['file_stems'][:5],
        'sample_dirs': signals['dir_names'][:3],
        'detected_extensions': sorted(list(detected_exts)),
        'matched_rules': matched_rules
    }

class TorrentFeatureExtractor(BaseEstimator, TransformerMixin):
    def __init__(self, max_features: int = 250000, normalize_dense: bool = True, dense_version: int = 3):
        self.max_features = max_features
        self.normalize_dense = normalize_dense
        self.dense_version = dense_version
        self.char_vectorizer = TfidfVectorizer(
            analyzer='char_wb',
            ngram_range=(3, 5),
            min_df=2,
            max_features=max_features,
            sublinear_tf=True
        )
        self.word_vectorizer = TfidfVectorizer(
            analyzer='word',
            ngram_range=(1, 2),
            min_df=2,
            max_features=60000,
            token_pattern=r'(?u)\b\w+\b',
            stop_words=CORPUS_STOP_WORDS,
            sublinear_tf=True
        )

    def fit(self, records: List[Dict[str, Any]], y: Any = None) -> 'TorrentFeatureExtractor':
        texts = []
        norm = getattr(self, 'normalize_dense', False)
        dv = getattr(self, 'dense_version', 3)
        for r in records:
            if isinstance(r, dict) and 'clean_text' in r and dv <= 2:
                txt = r['clean_text']
            else:
                txt, _ = get_text_and_features(r, normalize_dense=norm, dense_version=dv)
            texts.append(txt)
        self.char_vectorizer.fit(texts)
        self.word_vectorizer.fit(texts)
        return self

    def transform(self, records: List[Dict[str, Any]]) -> sp.csr_matrix:
        texts = []
        dense_list = []
        norm = getattr(self, 'normalize_dense', False)
        dv = getattr(self, 'dense_version', 3)
        for r in records:
            if isinstance(r, dict) and 'clean_text' in r and 'dense_vector' in r and dv <= 2:
                txt = r['clean_text']
                dense = r['dense_vector']
            else:
                txt, dense = get_text_and_features(r, normalize_dense=norm, dense_version=dv)
            texts.append(txt)
            dense_list.append(dense)

        X_char = self.char_vectorizer.transform(texts)
        X_word = self.word_vectorizer.transform(texts)
        X_dense = sp.csr_matrix(np.array(dense_list, dtype=np.float32))

        return sp.hstack([X_char, X_word, X_dense], format='csr')

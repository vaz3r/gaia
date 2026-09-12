import re
import math
import numpy as np
import scipy.sparse as sp
from sklearn.base import BaseEstimator, TransformerMixin
from sklearn.feature_extraction.text import TfidfVectorizer

# Regex patterns for domain scene rules
# Corpus-wide ubiquitous stop words discovered from 3.2M torrent entropy analysis
CORPUS_STOP_WORDS = [
    'the', 'and', 'of', 'in', 'for', 'to', 'by', 'on', 'all', 'me', 'my', 'it',
    'com', '2026', '2025', '2024', '2023', '2022', '2021', '2020', '2019',
    '01', '02', '03', '04', '05', '06', '07', '08', '09', '10', '11', '12',
    'www', 'org', 'net', 'rarbg', 'torrent', 'download'
]

# Regex patterns for domain scene rules (empirically expanded from 3.2M analysis)
RE_TV = re.compile(
    r'\b[sS]\d{1,2}[eE]\d{1,2}\b|\b\d{1,2}x\d{1,2}\b|\bseason\s*\d+\b|\bepisode\s*\d+\b|\bcomplete\s+series\b'
    r'|\bseries\s*\d+\b|\bpart\s*\d+\b|\bep\s*\d+\b|\[第\d+集\]|\[第\d+话\]|\[第\d+期\]|全\d+集|更新至'
    r'|\b(eztvx?|hdtv|dsnp|amzn|ddp5|s01|s02|s03|s04|s05|tgx|megusta)\b',
    re.IGNORECASE
)
RE_ANIME = re.compile(
    r'\[(horriblesubs|erai-raws|subsplease|judas|asw|commie|chyu|doki|hatsuyuki|kamigami|nekketsu|vcb-studio|'
    r'moozzi2|lolihouse|ember|coalgirls|reinforce|anime\s*time|baha|hr|ohys-raws|leopard-raws|nyaa)\]'
    r'|[\u3040-\u30ff\u31f0-\u31ff]'
    r'|\b(kimetsu|yaiba|jujutsu|kaisen|shingeki|one\s*piece|naruto|bleach|boku\s*no\s*hero|dragon\s*ball|'
    r'chainsaw\s*man|spy\s*x\s*family|frieren|dungeon\s*meshi|solo\s*leveling|oshi\s*no\s*ko|'
    r'rezero|isekai|shaman\s*king|yu-gi-oh|yugioh|gundam|evangelion|fullmetal|death\s*note|'
    r'gintama|haikyuu|jojo|tokyo\s*ghoul|vinland|mushoku\s*tensei|slime|danmachi|baki|berserk|'
    r'monogatari|konosuba|overlord|fate|boruto|inuyasha|dandadan|sousou|kaiju|wind\s*breaker|'
    r'dual[\s\-]audio|multi[\s\-]sub|multi[\s\-]audio|hi10p|10[\s\-]bit|nced|ncop|bdrip|anilibria|vostfr|multisub)\b',
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
    r'секс|порно|эротика|минет|шлюхи)\b'
    r'|\[ai\s*generated\]|\bai[\s\-_]*gen(erated)?\b'
    r'|\[\d{5,7}\]',
    re.IGNORECASE
)
RE_JAV = re.compile(
    r'\b[A-Za-z]{2,6}[-_ ]\d{3,5}\b'
    r'|\bFC2[-_ ]?(PPV)?[-_ ]?\d+\b'
    r'|\b(carib|1pon|10mu|heyd|c0930|h0930|h4610|siro|mkbd|s-cute)[-_ ]\d+\b'
    r'|\b(s1|moodyz|ideapocket|attackers|prestige|wanz|madonna|das|ebod|jul|ssis|ipx|mide|ssni|midv|stars|abw|cawd|dldss|meyd)[-_ ]\d+\b',
    re.IGNORECASE
)
RE_AUDIOBOOK = re.compile(
    r'(\b(audiobook|audio\s*book|narrat(ed|or)|unabridged|abridged|read\s*by|performed\s*by|voiced\s*by|'
    r'audible|audio\s*drama|full[\s\-]cast|\.m4b\b|\.aax\b|аудиокнига|читает|озвучка|'
    r'автор|исполнитель)\b|\b\d+h\d+m\b|\([A-Za-zА-Яа-я]+[_\s]+[A-Za-zА-Яа-я]\.?\))',
    re.IGNORECASE
)
RE_BOOK = re.compile(
    r'\b(epub|pdf|mobi|azw3|djvu|cbr|cbz|chm|retail\s*epub|ebook|e-book|course|tutorial|lecture|textbook|'
    r'udemy|coursera|masterclass|pluralsight|oreilly|packt|wiley|springer|cambridge|oxford|manual|'
    r'учебник|пособие|руководство|сборник|книга|guide|handbook)\b',
    re.IGNORECASE
)
RE_DOCU = re.compile(
    r'\b(bbc|pbs|national\s*geographic|nat\s*geo|discovery(\s*channel)?|docu|documentary|docuseries|'
    r'nature|history\s*channel|attenborough|planet\s*earth|blue\s*planet|curiositystream|novafilm|imax|mvgroup)\b',
    re.IGNORECASE
)
RE_GAME = re.compile(
    r'\b(fitgirl|dodi|repack|codex|skidrow|flt|plaza|cso|nsp|xci|playstation|ps4|ps5|ps3|ps2|psx|switch|'
    r'xbox|nintendo|roms?|reloaded|cpy|rune|tenoke|empress|razor1911|elamigos|gog|tinyiso|pc\s*game|'
    r'steamrip|cracked|kaos|rg\s*mechanics|deluxe\s*edition|definitive\s*edition|'
    r'multi\d+|v\d+(\.\d+)+|build\s*\d+|update\s*v?\d+|dlc|repacks?|patch-?fr)\b',
    re.IGNORECASE
)
RE_APP = re.compile(
    r'\b(adobe|autodesk|microsoft\s*office|windows\s*1\d|macos|crack|keygen|keymaker|patcher?|portable|setup|'
    r'multilingual|v\d+\.\d+|\.dmg\b|\.apk\b|winrar|installer|activator|x64|x86|win64|win32|loader|'
    r'coreldraw|solidworks|cyberlink|corel|acronis|vmware|jetbrains)\b',
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

EXT_CATEGORIES = {
    'video': {'mkv', 'mp4', 'avi', 'ts', 'wmv', 'vob', 'm4v', 'webm', 'mpg', 'mpeg', 'flv', 'mov', '3gp', 'm2ts'},
    'audio': {'flac', 'mp3', 'm4a', 'aac', 'wav', 'alac', 'ogg', 'ape', 'opus', 'wma', 'cue'},
    'audiobook': {'m4b', 'aax', 'aa'},
    'ebook': {'pdf', 'epub', 'mobi', 'azw3', 'djvu', 'fb2', 'cbr', 'cbz', 'chm'},
    'archive': {'zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'iso', 'bin'},
    'software': {'exe', 'msi', 'dmg', 'pkg', 'deb', 'rpm', 'appimage', 'apk'},
    'game_rom': {'nsp', 'xci', 'cia', 'vpk', 'nds', '3ds', 'wbfs', 'gcm', 'rom', 'wad'},
}

def extract_extension(filename):
    if not filename:
        return ''
    parts = filename.strip().rsplit('.', 1)
    if len(parts) > 1:
        ext = parts[1].lower()
        if len(ext) <= 6 and ext.isalnum():
            return ext
    return ''

def get_text_and_features(item, normalize_dense=True, dense_version=1):
    name = str(item.get('name') or '')
    total_size = float(item.get('total_size') or 0.0)
    file_count = float(item.get('file_count') or 1.0)
    files = item.get('files')
    
    file_paths = []
    ext_byte_shares = {cat: 0.0 for cat in EXT_CATEGORIES}
    has_files_list = False
    max_file_len = 0.0
    has_nfo = 0.0
    has_cue = 0.0
    has_setup_exe = 0.0
    
    max_inspect_files = 100 if dense_version >= 2 else 40
    
    if files:
        if isinstance(files, str):
            try:
                import json
                files = json.loads(files)
            except Exception:
                files = []
        if isinstance(files, list) and files:
            has_files_list = True
            for f in files[:max_inspect_files]:
                if isinstance(f, dict):
                    length = float(f.get('length') or 0.0)
                    if length > max_file_len:
                        max_file_len = length
                    path = f.get('path')
                    if isinstance(path, list) and path:
                        p_str = "/".join(str(p) for p in path)
                        file_paths.append(p_str)
                        ext = extract_extension(path[-1])
                        for cat, cat_exts in EXT_CATEGORIES.items():
                            if ext in cat_exts:
                                ext_byte_shares[cat] += length
                        if ext == 'nfo':
                            has_nfo = 1.0
                        elif ext == 'cue':
                            has_cue = 1.0
                        elif ext == 'exe' and 'setup' in path[-1].lower():
                            has_setup_exe = 1.0
    
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
    
    combined_name = name + " " + " ".join(file_paths[:5])
    
    has_adult = bool(RE_ADULT.search(combined_name))
    has_anime = bool(RE_ANIME.search(combined_name))
    has_docu = bool(RE_DOCU.search(combined_name))
    has_tv = bool(RE_TV.search(combined_name)) and (not has_anime) and (not has_docu)
    has_audiobook = bool(RE_AUDIOBOOK.search(combined_name))
    has_book = bool(RE_BOOK.search(combined_name))
    has_game = bool(RE_GAME.search(combined_name))
    has_app = bool(RE_APP.search(combined_name)) and (not has_game)
    has_movie = bool(RE_MOVIE.search(combined_name)) and (not has_tv) and (not has_docu) and (not has_anime)
    has_music = bool(RE_MUSIC.search(combined_name)) and (not has_audiobook)
    
    regex_feats = [
        1.0 if has_tv else 0.0,
        1.0 if has_anime else 0.0,
        1.0 if has_adult else 0.0,
        1.0 if has_audiobook else 0.0,
        1.0 if has_book else 0.0,
        1.0 if has_docu else 0.0,
        1.0 if has_game else 0.0,
        1.0 if has_app else 0.0,
        1.0 if has_movie else 0.0,
        1.0 if has_music else 0.0,
    ]
    
    if dense_version >= 2:
        largest_file_ratio = min(max_file_len / denom, 1.0)
        meta_feats = [
            largest_file_ratio,
            1.0 if has_files_list else 0.0,
            has_nfo,
            has_cue,
            has_setup_exe
        ]
        dense_vector = [eff_log_size, eff_log_count, is_single] + ext_ratios + regex_feats + meta_feats
        # Select informative file paths for text
        selected_paths = file_paths[:20] if len(file_paths) <= 20 else file_paths[:10] + file_paths[-10:]
        text_content = name + " " + " ".join(selected_paths)
    else:
        dense_vector = [eff_log_size, eff_log_count, is_single] + ext_ratios + regex_feats
        text_content = name + " " + " ".join(file_paths[:15])

    clean_text = re.sub(r'[\._\-\+\[\]\(\)\{\}]', ' ', text_content)
    clean_text = re.sub(r'\s+', ' ', clean_text).strip()
    
    return clean_text, dense_vector

def explain_features(item):
    """Returns human-readable explanation of features that fired for UI."""
    name = str(item.get('name') or '')
    total_size = float(item.get('total_size') or 0.0)
    file_count = int(item.get('file_count') or 1)
    files = item.get('files')
    
    size_mb = total_size / (1024 * 1024)
    size_str = f"{size_mb / 1024:.2f} GB" if size_mb >= 1024 else f"{size_mb:.1f} MB"
    
    detected_exts = set()
    name_ext = extract_extension(name)
    if name_ext:
        detected_exts.add('.' + name_ext)
        
    if files:
        if isinstance(files, str):
            try:
                import json
                files = json.loads(files)
            except Exception:
                files = []
        if isinstance(files, list):
            for f in files[:20]:
                if isinstance(f, dict) and 'path' in f and f['path']:
                    ext = extract_extension(f['path'][-1])
                    if ext:
                        detected_exts.add('.' + ext)

    matched_rules = []
    if RE_TV.search(name): matched_rules.append("TV Season/Episode pattern (SxxExx / Episode)")
    if RE_ANIME.search(name): matched_rules.append("Anime Fansub / Japanese characters")
    if RE_ADULT.search(name): matched_rules.append("Adult keyword / Studio marker")
    if RE_AUDIOBOOK.search(name): matched_rules.append("Audiobook narrator / Unabridged pattern")
    if RE_BOOK.search(name): matched_rules.append("Book document keyword (epub/pdf/mobi)")
    if RE_DOCU.search(name): matched_rules.append("Documentary network (BBC/PBS/Discovery/Nature)")
    if RE_GAME.search(name): matched_rules.append("Game repack / Console tag (FitGirl/CODEX/PS4/Switch)")
    if RE_APP.search(name): matched_rules.append("Application setup / Crack / Keygen")
    if RE_MOVIE.search(name): matched_rules.append("Movie release tag (Year + Resolution/Rip)")
    if RE_MUSIC.search(name): matched_rules.append("Music album / FLAC / 320kbps tag")

    return {
        "size_formatted": size_str,
        "file_count": file_count,
        "is_single_file": file_count <= 1,
        "detected_extensions": sorted(list(detected_exts)),
        "matched_rules": matched_rules
    }

class TorrentFeatureExtractor(BaseEstimator, TransformerMixin):
    def __init__(self, max_features=250000, normalize_dense=True, dense_version=2):
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
        
    def fit(self, records, y=None):
        texts = []
        norm = getattr(self, 'normalize_dense', False)
        dv = getattr(self, 'dense_version', 1)
        for r in records:
            if isinstance(r, dict) and 'clean_text' in r:
                txt = r['clean_text']
            else:
                txt, _ = get_text_and_features(r, normalize_dense=norm, dense_version=dv)
            texts.append(txt)
        self.char_vectorizer.fit(texts)
        self.word_vectorizer.fit(texts)
        return self
        
    def transform(self, records):
        texts = []
        dense_list = []
        norm = getattr(self, 'normalize_dense', False)
        dv = getattr(self, 'dense_version', 1)
        for r in records:
            if isinstance(r, dict) and 'clean_text' in r and 'dense_vector' in r:
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

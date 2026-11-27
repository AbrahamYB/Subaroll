// Normalize language metadata for display; unknown labels are kept as supplied.
const subtitleLanguages = (() => {
  const languages = [
    ['en', 'eng', 'English', 'english'], ['es', 'spa', 'Spanish', 'español', 'castellano'],
    ['pl', 'pol', 'Polish', 'polski'], ['ru', 'rus', 'Russian', 'русский'],
    ['fr', 'fra', 'French', 'fre', 'français'], ['de', 'deu', 'German', 'ger', 'deutsch'],
    ['it', 'ita', 'Italian', 'italiano'], ['pt', 'por', 'Portuguese', 'português'],
    ['nl', 'nld', 'Dutch', 'dut', 'nederlands'], ['uk', 'ukr', 'Ukrainian', 'українська'],
    ['cs', 'ces', 'Czech', 'cze', 'čeština'], ['sk', 'slk', 'Slovak', 'slo', 'slovenčina'],
    ['ro', 'ron', 'Romanian', 'rum', 'română'], ['hu', 'hun', 'Hungarian', 'magyar'],
    ['sv', 'swe', 'Swedish', 'svenska'], ['no', 'nor', 'Norwegian', 'norsk'],
    ['da', 'dan', 'Danish', 'dansk'], ['fi', 'fin', 'Finnish', 'suomi'],
    ['el', 'ell', 'Greek', 'gre', 'ελληνικά'], ['tr', 'tur', 'Turkish', 'türkçe'],
    ['bg', 'bul', 'Bulgarian', 'български'], ['hr', 'hrv', 'Croatian', 'hrvatski'],
    ['sr', 'srp', 'Serbian', 'српски', 'srpski'], ['sl', 'slv', 'Slovenian', 'slovenščina'],
    ['et', 'est', 'Estonian', 'eesti'], ['lv', 'lav', 'Latvian', 'latviešu'],
    ['lt', 'lit', 'Lithuanian', 'lietuvių'], ['is', 'isl', 'Icelandic', 'ice', 'íslenska'],
    ['ga', 'gle', 'Irish', 'gaeilge'], ['ca', 'cat', 'Catalan', 'català'],
  ];
  const lookup = new Map();
  for (const [code, iso3, english, ...aliases] of languages) {
    for (const alias of [code, iso3, english, ...aliases]) lookup.set(alias.toLocaleLowerCase(), english);
  }
  function englishName(label) {
    const text = String(label || '').trim();
    if (!text || /^(und|unknown)$/i.test(text)) return 'Unknown';
    const key = text.toLocaleLowerCase().replace(/_/g, '-');
    return lookup.get(key) || lookup.get(key.split('-')[0]) || text;
  }
  function recognized(label) {
    const key = String(label || '').trim().toLocaleLowerCase().replace(/_/g, '-');
    return lookup.get(key) || lookup.get(key.split('-')[0]) || null;
  }
  return { englishName, recognized };
})();

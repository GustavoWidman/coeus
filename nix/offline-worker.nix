# The offline page Worker as one ES module, for norn's `cloudflare.fleet.workers.<name>.module`.
# norn requires every machine to declare the same module, and the machines span darwin and linux,
# so the module is built from plain strings with builtins.toFile: no derivation, no system, and
# the same bytes and store path everywhere.
{ hosts }:
let
  inherit (builtins)
    concatStringsSep
    convertHash
    hashString
    length
    map
    match
    readFile
    replaceStrings
    toFile
    toJSON
    ;

  host =
    name:
    if match "[a-z0-9]([a-z0-9-]*[a-z0-9])?" name != null then
      "<li><i class=\"dot\"></i><span>${name}</span><span>no connector</span></li>"
    else
      throw "offlineWorker: `${name}` is not a host name";

  style = readFile ../web/src/lib/ui/tokens.css + readFile ../worker/offline.css;
  script = readFile ../worker/offline.js;

  icon =
    "data:image/svg+xml,"
    + replaceStrings [ "#" "<" ">" "\"" "\n" ] [ "%23" "%3C" "%3E" "'" "" ] (
      readFile ../web/static/favicon.svg
    );

  sha256 =
    text:
    "'sha256-${
      convertHash {
        hash = hashString "sha256" text;
        hashAlgo = "sha256";
        toHashFormat = "base64";
      }
    }'";

  # the page's own fetch to see whether a host is back is the only request it makes
  csp = concatStringsSep "; " [
    "default-src 'none'"
    "style-src ${sha256 style}"
    "script-src ${sha256 script}"
    "img-src data:"
    "connect-src 'self'"
    "base-uri 'none'"
    "form-action 'none'"
    "frame-ancestors 'none'"
  ];

  page =
    replaceStrings
      [ "@style@" "@script@" "@icon@" "@count@" "@hosts@" ]
      [
        "<style>${style}</style>"
        "<script>${script}</script>"
        icon
        (toString (length hosts))
        (concatStringsSep "" (map host hosts))
      ]
      (readFile ../worker/offline.html);
in
toFile "coeus-offline-worker.js" (
  replaceStrings [ "__PAGE__" "__CSP__" ] [ (toJSON page) (toJSON csp) ] (
    readFile ../worker/worker.js
  )
)

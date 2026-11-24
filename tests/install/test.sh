#!/bin/sh
# Runs install.sh against a fake release, with no network: a stub curl serves
# the files. Checks a clean install and that bad checksums are refused.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case $(uname -s) in
Linux) os=unknown-linux-musl ;;
Darwin) os=apple-darwin ;;
*) echo "unsupported system" >&2; exit 1 ;;
esac
case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "unsupported machine" >&2; exit 1 ;;
esac
name=arena-v9.9.9-$arch-$os
asset=$name.tar.gz

mkdir -p "$work/bin" "$work/release" "$work/$name/man" "$work/$name/completions"
printf '#!/bin/sh\necho arena 9.9.9\n' >"$work/$name/arena"
chmod +x "$work/$name/arena"
touch "$work/$name/man/arena.1" "$work/$name/completions/arena.bash" \
    "$work/$name/completions/_arena" "$work/$name/completions/arena.fish"
tar czf "$work/release/$asset" -C "$work" "$name"

if command -v sha256sum >/dev/null 2>&1; then
    sum=$(sha256sum "$work/release/$asset" | cut -d' ' -f1)
else
    sum=$(shasum -a 256 "$work/release/$asset" | cut -d' ' -f1)
fi

# Serves https://.../FILE from the fake release directory.
cat >"$work/bin/curl" <<EOF
#!/bin/sh
out=
while [ \$# -gt 1 ]; do
    if [ "\$1" = -o ]; then out=\$2; shift; fi
    shift
done
exec cp "$work/release/\${1##*/}" "\$out"
EOF
chmod +x "$work/bin/curl"

install_to() {
    env PATH="$work/bin:$PATH" VERSION=9.9.9 PREFIX="$work/$1" sh "$root/install.sh"
}

echo "$sum  $asset" >"$work/release/SHA256SUMS"
install_to good >/dev/null
[ "$("$work/good/bin/arena" --version)" = "arena 9.9.9" ]
[ -f "$work/good/share/man/man1/arena.1" ]
[ -f "$work/good/share/bash-completion/completions/arena" ]
[ -f "$work/good/share/zsh/site-functions/_arena" ]
[ -f "$work/good/share/fish/vendor_completions.d/arena.fish" ]
echo "ok    clean install"

echo "0000000000000000000000000000000000000000000000000000000000000000  $asset" \
    >"$work/release/SHA256SUMS"
if install_to tampered >/dev/null 2>&1 || [ -e "$work/tampered/bin/arena" ]; then
    echo "FAIL  installed despite a checksum mismatch" >&2
    exit 1
fi
echo "ok    checksum mismatch refused"

echo "$sum  another-file.tar.gz" >"$work/release/SHA256SUMS"
if install_to unlisted >/dev/null 2>&1 || [ -e "$work/unlisted/bin/arena" ]; then
    echo "FAIL  installed an asset missing from SHA256SUMS" >&2
    exit 1
fi
echo "ok    unlisted asset refused"

#!/bin/sh
# Downloads the arena CLI for this platform, verifies its checksum and installs it.
#
#   curl -fsSL https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.sh | sh
#
# PREFIX   install prefix (default /usr/local; sudo is used if it isn't writable)
# VERSION  release to install, e.g. 0.6.0 (default: the latest)
set -eu

repo=hugoocoto/runtime-arena

die() {
    echo "error: $*" >&2
    exit 1
}

if [ -n "${PREFIX:-}" ]; then
    prefix=$PREFIX
else
    prefix=/usr/local
fi

for tool in curl tar install; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done

case $(uname -s) in
Linux) os=unknown-linux-musl ;;
Darwin) os=apple-darwin ;;
*) die "no prebuilt CLI for $(uname -s); download one from https://github.com/$repo/releases" ;;
esac
case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) die "no prebuilt CLI for $(uname -m)" ;;
esac
target=$arch-$os

if [ -n "${VERSION:-}" ]; then
    tag=v${VERSION#v}
else
    # Asset names contain the tag, so find it from the "latest" redirect. The
    # GitHub API would rate-limit a room of participants behind one address.
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") ||
        die "cannot find the latest release"
    tag=${url##*/}
fi
case $tag in
v[0-9]*) ;;
*) die "cannot find the latest release" ;;
esac

name=arena-$tag-$target
asset=$name.tar.gz
base=https://github.com/$repo/releases/download/$tag
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "Downloading $name"
curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS" ||
    die "release $tag has no SHA256SUMS, so it can't be verified; this installer needs a newer release"
curl -fsSL -o "$work/$asset" "$base/$asset" || die "cannot download $base/$asset"

awk -v file="$asset" '$2 == file' "$work/SHA256SUMS" >"$work/expected"
[ -s "$work/expected" ] || die "SHA256SUMS has no entry for $asset"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$work" && sha256sum -c expected >/dev/null) || die "checksum mismatch for $asset"
elif command -v shasum >/dev/null 2>&1; then
    (cd "$work" && shasum -a 256 -c expected >/dev/null) || die "checksum mismatch for $asset"
else
    die "sha256sum or shasum is required to verify the download"
fi

tar xzf "$work/$asset" -C "$work"

# The closest existing parent decides whether sudo is needed.
parent=$prefix
while [ ! -e "$parent" ]; do
    parent=$(dirname "$parent")
done
if [ -w "$parent" ]; then
    sudo=
elif command -v sudo >/dev/null 2>&1; then
    sudo=sudo
else
    die "cannot write to $prefix; set PREFIX to a directory you own"
fi

# The same layout as the archive's Makefile, which a clean macOS can't run:
# its `make` is a stub until the Command Line Tools are installed.
src=$work/$name
share=$prefix/share
$sudo install -d "$prefix/bin" "$share/man/man1" "$share/bash-completion/completions" \
    "$share/zsh/site-functions" "$share/fish/vendor_completions.d"
$sudo install -m 755 "$src/arena" "$prefix/bin/arena"
$sudo install -m 644 "$src"/man/*.1 "$share/man/man1/"
$sudo install -m 644 "$src/completions/arena.bash" "$share/bash-completion/completions/arena"
$sudo install -m 644 "$src/completions/_arena" "$share/zsh/site-functions/_arena"
$sudo install -m 644 "$src/completions/arena.fish" "$share/fish/vendor_completions.d/arena.fish"

echo "Installed arena $tag to $prefix/bin/arena"
case :$PATH: in
*:"$prefix/bin":*)
    "$prefix/bin/arena" --version
    ;;
*)
    echo "Add it to your PATH: export PATH=\"$prefix/bin:\$PATH\""
    echo "Then verify it with: arena --version"
    ;;
esac
case ${SHELL:-} in
*/zsh) echo "For zsh completions, add before compinit: fpath=(\"$share/zsh/site-functions\" \$fpath)" ;;
esac

#!/usr/bin/env bash
# One signed app commit per architecture, shared by the channel and downloads.
# stage uses only the existing builder's repository and imported signing key.
# assemble authenticates both handoffs before exporting either public output.
set -euo pipefail

fail() { printf 'release-artifacts: %s\n' "$*" >&2; exit 2; }

check_release() {
    [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.+-]+)?$ ]] || fail 'invalid version'
    [[ $source =~ ^[0-9a-f]{40}$ ]] || fail 'invalid source revision'
    [[ $fingerprint =~ ^[0-9A-Fa-f]{40}$ ]] || fail 'invalid signing fingerprint'
}

check_arch() {
    case $arch in x86_64|aarch64) ;; *) fail 'unsupported release architecture' ;; esac
}

read_commits() {
    app_ref=app/dev.harding.Kjerag/$arch/stable
    debug_ref=runtime/dev.harding.Kjerag.Debug/$arch/stable
    app_commit=$(<"$repository/refs/heads/$app_ref")
    debug_commit=$(<"$repository/refs/heads/$debug_ref")
    [[ $app_commit =~ ^[0-9a-f]{64}$ ]] || fail "invalid app ref for $arch"
    [[ $debug_commit =~ ^[0-9a-f]{64}$ ]] || fail "invalid debug ref for $arch"
}

provenance() {
    printf 'format=1\nsource=%s\nversion=%s\narch=%s\napp=%s\ndebug=%s\n' \
        "$source" "$version" "$arch" "$app_commit" "$debug_commit"
}

new_output() {
    # Never reuse a partial run or mix artifacts from different source tags.
    # mkdir deliberately fails if the exact destination already exists.
    mkdir -- "$output"
    output=$(realpath -- "$output")
}

stage() {
    [[ $# == 6 ]] || fail 'usage: release-artifacts.sh stage REPOSITORY OUTPUT ARCH VERSION SOURCE FINGERPRINT'
    repository=$(realpath -e -- "$1")
    output=$2 arch=$3 version=$4 source=$5 fingerprint=$6
    check_release
    check_arch
    case $(realpath -m -- "$output")/ in
        "$repository/"*) fail 'stage output must be outside the input repository' ;;
    esac
    read_commits
    new_output
    # The repository only: never artifact the builder state, GPG home or key.
    cp -a -- "$repository" "$output/repository"
    provenance > "$output/provenance.txt"
    gpg --batch --local-user "$fingerprint" --detach-sign \
        --output "$output/provenance.sig" "$output/provenance.txt"
    printf 'Staged %s: %s\n' "$app_ref" "$app_commit"
}

assemble() {
    [[ $# == 5 ]] || fail 'usage: release-artifacts.sh assemble STAGING OUTPUT VERSION SOURCE FINGERPRINT'
    staging=$(realpath -e -- "$1")
    output=$2 version=$3 source=$4 fingerprint=$5
    check_release
    case $(realpath -m -- "$output")/ in
        "$staging/"*) fail 'assembly output must be outside staging' ;;
    esac
    new_output
    mkdir -- "$output/verification" "$output/bundles"
    chmod 700 "$output/verification"
    # Export only the explicitly selected public key. Artifact-supplied keys
    # never decide which signatures the publication job trusts.
    gpg --batch --export "$fingerprint" > "$output/signing-key.gpg"
    test -s "$output/signing-key.gpg"

    # Verify every signed provenance record before importing any architecture.
    # It binds the expected source/tag/architecture to both exact commit IDs.
    local -A apps debug
    for arch in x86_64 aarch64; do
        handoff=$staging/release-$arch
        gpgv --homedir "$output/verification" --keyring "$output/signing-key.gpg" \
            "$handoff/provenance.sig" "$handoff/provenance.txt"
        repository=$handoff/repository
        read_commits
        provenance > "$output/verification/expected-$arch.txt"
        cmp -- "$output/verification/expected-$arch.txt" "$handoff/provenance.txt"
        apps[$arch]=$app_commit
        debug[$arch]=$debug_commit
    done

    destination=$output/repository
    ostree --repo="$destination" init --mode=archive-z2
    for arch in x86_64 aarch64; do
        repository=$staging/release-$arch/repository
        app_ref=app/dev.harding.Kjerag/$arch/stable
        debug_ref=runtime/dev.harding.Kjerag.Debug/$arch/stable
        app_commit=${apps[$arch]}
        debug_commit=${debug[$arch]}
        remote=release-$arch
        ostree --repo="$destination" remote add \
            --gpg-import="$output/signing-key.gpg" "$remote" "file://$repository"
        # Pull exactly the two authenticated IDs, even if an input ref changes
        # after verification. Check object checksums and both commit signatures.
        ostree --repo="$destination" pull-local --untrusted --gpg-verify \
            --remote="$remote" "$repository" "$app_commit" "$debug_commit"
        ostree --repo="$destination" refs --create="$app_ref" "$app_commit"
        ostree --repo="$destination" refs --create="$debug_ref" "$debug_commit"
        ostree --repo="$destination" remote delete "$remote"
        printf '%s %s\n%s %s\n' "$app_ref" "$app_commit" "$debug_ref" "$debug_commit" \
            >> "$output/commits.txt"
    done
    flatpak build-update-repo --gpg-sign="$fingerprint" "$destination"

    for arch in x86_64 aarch64; do
        bundle=kjerag-$version-$arch.flatpak
        flatpak build-bundle --arch="$arch" \
            --repo-url=https://kjerag.harding.dev/ \
            --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
            --gpg-keys="$output/signing-key.gpg" \
            "$destination" "$output/bundles/$bundle" dev.harding.Kjerag stable
        (cd "$output/bundles" && sha256sum "$bundle" > "$bundle.sha256")
    done
    printf 'Both signed architectures staged for %s at %s\n' "$version" "$source"
}

release_record() {
    printf 'format=1\nsource=%s\nversion=%s\n' "$source" "$version"
    cat "$output/commits.txt"
}

payload_hashes() {
    # These are the only publishable paths. Refuse symlinks: Actions artifacts
    # must not resolve a link into unrelated files or alter its meaning.
    (
        cd "$output"
        test ! -L commits.txt && test ! -L release.txt || fail 'publication record contains a symlink'
        test -z "$(find repository bundles -type l -print -quit)" || fail 'publication contains a symlink'
        { printf '%s\0' commits.txt release.txt; find repository bundles -type f -print0; } |
            LC_ALL=C sort -z | xargs -0 sha256sum
    )
}

verify_commits() {
    local expected
    expected=$(for arch in x86_64 aarch64; do
        repository=$output/repository
        read_commits
        printf '%s %s\n%s %s\n' "$app_ref" "$app_commit" "$debug_ref" "$debug_commit"
    done)
    test "$expected" = "$(cat "$output/commits.txt")" || fail 'published refs differ from assembled commits'
    for arch in x86_64 aarch64; do
        (cd "$output/bundles" && sha256sum --check "kjerag-$version-$arch.flatpak.sha256")
    done
}

seal() {
    [[ $# == 4 ]] || fail 'usage: release-artifacts.sh seal OUTPUT VERSION SOURCE FINGERPRINT'
    output=$(realpath -e -- "$1")
    version=$2 source=$3 fingerprint=$4
    check_release
    test ! -e "$output/payload.sig" || fail 'publication is already sealed'
    verify_commits
    release_record > "$output/release.txt"
    cp -- "$output/release.txt" "$output/repository/kjerag-release.txt"
    gpg --batch --local-user "$fingerprint" --detach-sign \
        --output "$output/repository/kjerag-release.sig" "$output/repository/kjerag-release.txt"
    payload_hashes > "$output/payload.sha256"
    gpg --batch --local-user "$fingerprint" --detach-sign \
        --output "$output/payload.sig" "$output/payload.sha256"
    printf 'Sealed both download bundles and the complete channel repository.\n'
}

verify() {
    [[ $# == 4 ]] || fail 'usage: release-artifacts.sh verify OUTPUT VERSION SOURCE FINGERPRINT'
    output=$(realpath -e -- "$1")
    version=$2 source=$3 fingerprint=$4
    check_release
    local verification
    verification=$(mktemp -d "$output/verification.XXXXXXXX")
    gpg --batch --export "$fingerprint" > "$verification/key.gpg"
    test -s "$verification/key.gpg"
    gpgv --homedir "$verification" --keyring "$verification/key.gpg" \
        "$output/payload.sig" "$output/payload.sha256"
    # Recompute the complete selected file set, not sha256sum -c on untrusted
    # path names. Added, missing and changed payload files all fail closed.
    payload_hashes > "$verification/actual.sha256"
    cmp -- "$verification/actual.sha256" "$output/payload.sha256"
    release_record > "$verification/release.txt"
    cmp -- "$verification/release.txt" "$output/release.txt"
    cmp -- "$output/release.txt" "$output/repository/kjerag-release.txt"
    gpgv --homedir "$verification" --keyring "$verification/key.gpg" \
        "$output/repository/kjerag-release.sig" "$output/repository/kjerag-release.txt"
    verify_commits
    printf 'Verified complete publication for %s at %s\n' "$version" "$source"
}

command=${1:-}
case $command in
    stage) shift; stage "$@" ;;
    assemble) shift; assemble "$@" ;;
    seal) shift; seal "$@" ;;
    verify) shift; verify "$@" ;;
    *) fail 'choose stage, assemble, seal or verify' ;;
esac

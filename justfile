publish:
    cargo publish --package guilderia-parser
    cargo publish --package guilderia-config
    cargo publish --package guilderia-result
    cargo publish --package guilderia-files
    cargo publish --package guilderia-permissions
    cargo publish --package guilderia-models
    cargo publish --package guilderia-presence
    cargo publish --package guilderia-database

patch:
    cargo release version patch --execute

minor:
    cargo release version minor --execute

major:
    cargo release version major --execute

release:
    scripts/try-tag-and-release.sh

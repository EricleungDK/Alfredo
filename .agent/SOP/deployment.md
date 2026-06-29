# Packaging & Distribution

**Last Updated**: 
## Build Tool



## Platforms

| Platform | Format | Command |
|----------|--------|---------|
| Windows | NSIS installer | `npm run build:win` |
| macOS | DMG | `npm run build:mac` |
| Linux | AppImage / deb | `npm run build:linux` |

## Pre-Build Checklist

## Auto-Update


## Code Signing

- Windows: requires code signing certificate for trusted installs
- macOS: requires Apple Developer certificate + notarization
- Linux: no signing needed for AppImage

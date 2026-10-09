"""Collect installed dependency notices without fetching or including runtime data."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]

def license_files(directory):
    files = []
    for entry in directory.iterdir():
        name = entry.name.lower()
        if name.startswith(('license', 'licence', 'copying', 'notice', 'copyright')):
            if entry.is_file():
                files.append(entry)
            elif entry.is_dir():
                files.extend(p for p in entry.rglob('*') if p.is_file())
    return sorted(files)

def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
         '--filter-platform', 'x86_64-pc-windows-msvc'], cwd=ROOT, encoding='utf-8'))
    entries = []
    for package in metadata['packages']:
        if package['source'] is None:
            continue
        entries.append((f"Rust: {package['name']} {package['version']}",
                        package.get('license') or 'See license text',
                        package.get('repository') or package['source'],
                        Path(package['manifest_path']).parent))
    modules = ROOT / 'apps/desktop/node_modules/.pnpm'
    for manifest in modules.glob('*/node_modules/*/package.json'):
        package = json.loads(manifest.read_text(encoding='utf-8'))
        entries.append((f"JavaScript: {package['name']} {package['version']}",
                        str(package.get('license', 'See license text')),
                        f"https://www.npmjs.com/package/{package['name']}", manifest.parent))
    for manifest in modules.glob('*/node_modules/@*/*/package.json'):
        package = json.loads(manifest.read_text(encoding='utf-8'))
        entries.append((f"JavaScript: {package['name']} {package['version']}",
                        str(package.get('license', 'See license text')),
                        f"https://www.npmjs.com/package/{package['name']}", manifest.parent))
    output = [
        'Cursor Sub2API BYOK 0.1.0 — THIRD-PARTY NOTICES',
        'Developed and maintained by MicroEduLab — https://microedulab.com/',
        'Development repository: https://github.com/dude1wudv/cursor-sub2api-byok',
        'Fork of https://github.com/leookun/cursor-byok',
        'Upstream commit: 7ee68c2b7fef66a0e0279273d037d23fbc2f11ad',
        'Original MIT license and copyright are provided in LICENSE.',
        'This inventory conservatively includes installed build dependencies as well as runtime dependencies.',
        'System Microsoft Edge WebView2 Runtime is separately installed and licensed by Microsoft.',
        'WebView2: https://developer.microsoft.com/microsoft-edge/webview2/',
        '',
    ]
    seen = set()
    for title, declared, source, directory in sorted(entries):
        if title in seen:
            continue
        seen.add(title)
        output.extend(['=' * 78, title, f'Declared license: {declared}', f'Source: {source}', ''])
        for file in license_files(directory):
            output.extend([f'--- {file.relative_to(directory)} ---', file.read_text(encoding='utf-8', errors='replace'), ''])
    destination = Path(sys.argv[1]) / 'THIRD-PARTY-NOTICES.txt'
    destination.write_text('\n'.join(output), encoding='utf-8')
    print(f'Collected {len(seen)} dependency notices.')

if __name__ == '__main__':
    main()

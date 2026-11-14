#!/usr/bin/env python3
"""CharlotteOS SDK command. Copied to the SDK root as `charlotte`."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib


def checked_sdk(root):
    lock = json.loads((root / 'sdk.json').read_text())
    for name, expected in lock['files'].items():
        path = root / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f'SDK input differs from sdk.json: {name}')
    return lock


def execute(command, cwd):
    subprocess.run(command, cwd=cwd, check=True)


def checked_project_sources(sdk, project):
    """Known platform dependencies must come from this exact source closure."""
    packages = {p.parent.name for p in (sdk / 'crates').glob('*/Cargo.toml')}
    manifests = [project / 'Cargo.toml']
    root = tomllib.loads(manifests[0].read_text())
    for member in root.get('workspace', {}).get('members', []):
        manifests.extend(p / 'Cargo.toml' for p in project.glob(member))

    def visit(table, manifest):
        if not isinstance(table, dict):
            return
        for key, value in table.items():
            if key in ('dependencies', 'dev-dependencies', 'build-dependencies'):
                for alias, dependency in value.items():
                    package = dependency.get('package', alias) if isinstance(dependency, dict) else alias
                    if package not in packages:
                        continue
                    if isinstance(dependency, dict) and dependency.get('workspace'):
                        continue  # The workspace table is checked by the same walk.
                    path = dependency.get('path') if isinstance(dependency, dict) else None
                    if not path or (manifest.parent / path).resolve() != (sdk / 'crates' / package).resolve():
                        raise ValueError(f'{manifest}: {package} must use the selected SDK source')
            else:
                visit(value, manifest)

    for manifest in manifests:
        visit(tomllib.loads(manifest.read_text()), manifest)


def new_project(sdk, project, lock):
    name = project.name.replace('_', '-')
    if not name or not name[0].isalpha() or not all(c.isascii() and (c.isalnum() or c == '-') for c in name):
        raise ValueError('project directory must be an ASCII Rust package name')
    if project.exists():
        raise ValueError(f'refusing to overwrite {project}')
    (project / 'app/src').mkdir(parents=True)
    (project / 'logic/src').mkdir(parents=True)
    sdk_dependency = json.dumps(str(sdk / 'crates/charlotte-app'))
    (project / 'Cargo.toml').write_text('[workspace]\nmembers = ["app", "logic"]\nresolver = "3"\n\n[profile.release]\npanic = "abort"\nlto = true\nopt-level = 3\n')
    (project / 'rust-toolchain.toml').write_text(f'[toolchain]\nchannel = "{lock["toolchain"]}"\ncomponents = ["rust-src", "llvm-tools"]\n')
    (project / 'app/Cargo.toml').write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2024"\n\n[[bin]]\nname = "{name}"\npath = "src/main.rs"\ntest = false\nbench = false\n\n[dependencies]\ncharlotte-app = {{ path = {sdk_dependency} }}\n{name}-logic = {{ path = "../logic" }}\n')
    (project / 'logic/Cargo.toml').write_text(f'[package]\nname = "{name}-logic"\nversion = "0.1.0"\nedition = "2024"\n')
    (project / 'logic/src/lib.rs').write_text('''#![no_std]
/// Example business rule: quantities must be nonzero and totals must fit.
pub fn total(quantity: u32, unit_price: u64) -> Option<u64> {
    if quantity == 0 { return None; }
    unit_price.checked_mul(u64::from(quantity))
}
#[cfg(test)]
mod tests {
    #[test]
    fn totals_reject_empty_orders_and_overflow() {
        assert_eq!(super::total(3, 25), Some(75));
        assert_eq!(super::total(0, 25), None);
        assert_eq!(super::total(2, u64::MAX), None);
    }
}
''')
    (project / 'app/src/main.rs').write_text(f'''#![no_std]
#![no_main]
use charlotte_app::{{AppExit, Application, Context, LaunchError}};
fn run(ctx: &Context) -> Result<AppExit, LaunchError> {{
    let _application = Application::from_context(ctx)?;
    let total = {name.replace('-', '_')}_logic::total(3, 25);
    charlotte_app::logln!("total: {{:?}}", total);
    Ok(AppExit::Completed)
}}
charlotte_app::entry!(run);
''')
    (project / 'charlotte.json').write_text(json.dumps({'name': name, 'sdk': str(sdk), 'sdk_fingerprint': hashlib.sha256((sdk / 'sdk.json').read_bytes()).hexdigest(), 'stack_pages': 4, 'max_threads': 1, 'shutdown_grace_ms': 5000, 'grants': []}, indent=2) + '\n')
    (project / '.gitignore').write_text('/target/\n/.charlotte/\n')
    shutil.copyfile(sdk / 'Cargo.lock', project / 'Cargo.lock')
    print(f'Created {project}; run {sdk / "charlotte"} test {project}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('doctor')
    p = sub.add_parser('new'); p.add_argument('project', type=Path)
    p = sub.add_parser('test'); p.add_argument('project', type=Path)
    p = sub.add_parser('build'); p.add_argument('project', type=Path); p.add_argument('--arch', choices=['aarch64','x86_64'], default='aarch64'); p.add_argument('--output', type=Path, required=True)
    p = sub.add_parser('package'); p.add_argument('project', type=Path); p.add_argument('--elf', type=Path, required=True); p.add_argument('--key-file', type=Path, required=True); p.add_argument('--version', type=int, required=True); p.add_argument('--rollback', type=int, required=True)
    args = parser.parse_args()
    sdk = Path(__file__).resolve().parent
    lock = checked_sdk(sdk)
    toolchain = lock['toolchain']
    if args.command == 'doctor':
        execute(['rustup', 'which', '--toolchain', toolchain, 'rustc'], sdk)
        print(f'SDK inputs verified; runtime ABI {lock["runtime_abi"]}; Sitas {lock["sitas_revision"]}')
        print('Target builds additionally require rust-src and llvm-tools.')
        return
    project = args.project.resolve()
    if args.command == 'new':
        new_project(sdk, project, lock); return
    plan = json.loads((project / 'charlotte.json').read_text())
    fingerprint = hashlib.sha256((sdk / 'sdk.json').read_bytes()).hexdigest()
    if plan['sdk_fingerprint'] != fingerprint or Path(plan['sdk']).resolve() != sdk:
        raise ValueError('project pins a different SDK; update its reviewed charlotte.json explicitly')
    checked_project_sources(sdk, project)
    cargo = ['cargo', f'+{toolchain}']
    if args.command == 'test':
        execute(cargo + ['test', '--manifest-path', str(project / 'Cargo.toml'), '-p', plan['name'] + '-logic'], project)
    elif args.command == 'build':
        execute([str(sdk / 'build-external-elf.sh'), '--manifest', str(project / 'app/Cargo.toml'), '--bin', plan['name'], '--arch', args.arch, '--output', str(args.output.resolve())], project)
    else:
        # Signer writes only to project-owned state, never to the SDK inputs.
        signer = project / ('.charlotte/signer-' + fingerprint[:16])
        if not signer.exists():
            shutil.copytree(sdk / 'signer', signer)
        execute(cargo + ['build', '--manifest-path', str(signer / 'Cargo.toml')], project)
        execute([str(signer / 'target/debug/cluster-sign'), 'elf-sign', str(args.elf.resolve()), plan['name'], str(args.key_file.resolve()), 'service', str(args.version), str(args.rollback), '0', '-'], project)
        print('Signed ELF; use deployment-sign to review and sign placement, grants and resource policy before deployment.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f'error: {error}', file=sys.stderr)
        sys.exit(1)

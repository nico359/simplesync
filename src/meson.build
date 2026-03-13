pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
moduledir = pkgdatadir / 'cloudsend'
gnome = import('gnome')

gnome.compile_resources('cloudsend',
  'cloudsend.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
)

python = import('python')

conf = configuration_data()
conf.set('PYTHON', python.find_installation('python3').full_path())
conf.set('VERSION', meson.project_version())
conf.set('localedir', get_option('prefix') / get_option('localedir'))
conf.set('pkgdatadir', pkgdatadir)

configure_file(
  input: 'cloudsend.in',
  output: 'cloudsend',
  configuration: conf,
  install: true,
  install_dir: get_option('bindir'),
  install_mode: 'rwxr-xr-x'
)

cloudsend_sources = [
  '__init__.py',
  'main.py',
  'window.py',
  'account_page.py',
  'targets_page.py',
  'target_edit_page.py',
  'remote_browser_page.py',
  'webdav.py',
  'push.py',
  'db.py',
  'keyring.py',
]

install_data(cloudsend_sources, install_dir: moduledir)

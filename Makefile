VERSION=0.1
PACKAGE=api-test-server
PKGREL=1


all: target/release/api-test-server

target/release/api-test-server:
	cargo build --release

deb:
	rm -rf build
	# build here to cache results
	cargo build --release
	rsync -a debian Cargo.lock Cargo.toml src target build
	cd build; dpkg-buildpackage -b -us -uc



clean:
	cargo clean
	rm -rf *.deb *.buildinfo *.changes build
	find . -name '*~' -exec rm {} ';'


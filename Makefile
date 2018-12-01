PACKAGE=api-test-server
PKGVER=0.1
PKGREL=1

ARCH:=$(shell dpkg-architecture -qDEB_BUILD_ARCH)
GITVERSION:=$(shell git rev-parse HEAD)


DEB=${PACKAGE}_${PKGVER}-${PKGREL}_${ARCH}.deb

DESTDIR=

all: target/release/api-test-server

target/release/api-test-server:
	cargo build --release

.PHONY: deb
deb ${DEB}:
	rm -rf build
	# build here to cache results
	cargo build --release
	rsync -a debian Cargo.lock Cargo.toml src www target build
	cd build; dpkg-buildpackage -b -us -uc


distclean: clean

clean:
	cargo clean
	rm -rf *.deb *.buildinfo *.changes build
	find . -name '*~' -exec rm {} ';'

.PHONY: dinstall
dinstall: ${DEB}
	dpkg -i ${DEB}

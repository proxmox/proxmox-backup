.. _sysadmin_package_repositories:

Debian Package Repositories
---------------------------

All Debian based systems use APT_ as a package management tool. The lists of
repositories are defined in ``/etc/apt/sources.list`` and the ``.list`` files found
in the ``/etc/apt/sources.d/`` directory. Updates can be installed directly
with the ``apt`` command-line tool, or via the GUI.

APT_ ``sources.list`` files list one package repository per line, with the most
preferred source listed first. Empty lines are ignored and a ``#`` character
anywhere on a line marks the remainder of that line as a comment. The
information available from the configured sources is acquired by ``apt
update``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://deb.debian.org/debian bookworm main contrib
  deb http://deb.debian.org/debian bookworm-updates main contrib

  # security updates
  deb http://security.debian.org/debian-security bookworm-security main contrib


In addition, you need a package repository from Proxmox to get Proxmox Backup
updates.

.. image:: images/screenshots/pbs-gui-administration-apt-repos.png
  :target: _images/pbs-gui-administration-apt-repos.png
  :align: right
  :alt: APT Repository Management in the Web Interface

.. _package_repos_secure_apt:

SecureApt
~~~~~~~~~

The `Release` files in the repositories are signed with GnuPG. APT is using
these signatures to verify that all packages are from a trusted source.

If you install Proxmox Backup Server from an official ISO image, the
verification key is already installed.

If you install Proxmox Backup Server on top of Debian, download and install the
key with the following commands:

.. code-block:: console

 # wget https://enterprise.proxmox.com/debian/proxmox-release-bookworm.gpg -O /etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg

Verify the SHA512 checksum afterwards with the expected output below:

.. code-block:: console

 # sha512sum /etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg
 7da6fe34168adc6e479327ba517796d4702fa2f8b4f0a9833f5ea6e6b48f6507a6da403a274fe201595edc86a84463d50383d07f64bdde2e3658108db7d6dc87  /etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg

and the md5sum, with the expected output below:

.. code-block:: console

 # md5sum /etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg
 41558dc019ef90bd0f6067644a51cf5b /etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg

.. _sysadmin_package_repos_enterprise:

`Proxmox Backup`_ Enterprise Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This is the stable, recommended repository. It is available for
all Proxmox Backup subscription users. It contains the most stable packages,
and is suitable for production use. The ``pbs-enterprise`` repository is
enabled by default:

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list.d/pbs-enterprise.list``

  deb https://enterprise.proxmox.com/debian/pbs bookworm pbs-enterprise


To never miss important security fixes, the superuser (``root@pam`` user) is
notified via email about new packages as soon as they are available. The
change-log and details of each package can be viewed in the GUI (if available).

Please note that you need a valid subscription key to access this
repository. More information regarding subscription levels and pricing can be
found at https://www.proxmox.com/en/proxmox-backup-server/pricing

.. note:: You can disable this repository by commenting out the above line
 using a `#` (at the start of the line). This prevents error messages if you do
 not have a subscription key. Please configure the ``pbs-no-subscription``
 repository in that case.


`Proxmox Backup`_ No-Subscription Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

As the name suggests, you do not need a subscription key to access
this repository. It can be used for testing and non-production
use. It is not recommended to use it on production servers, because these
packages are not always heavily tested and validated.

We recommend to configure this repository in ``/etc/apt/sources.list``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://deb.debian.org/debian bookworm main contrib
  deb http://deb.debian.org/debian bookworm-updates main contrib

  # Proxmox Backup Server pbs-no-subscription repository provided by proxmox.com,
  # NOT recommended for production use
  deb http://download.proxmox.com/debian/pbs bookworm pbs-no-subscription

  # security updates
  deb http://security.debian.org/debian-security bookworm-security main contrib


`Proxmox Backup`_ Test Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This repository contains the latest packages and is heavily used by developers
to test new features.

.. .. warning:: the ``pbstest`` repository should (as the name implies)
  only be used to test new features or bug fixes.

You can access this repository by adding the following line to
``/etc/apt/sources.list``:

.. code-block:: sources.list
  :caption: sources.list entry for ``pbstest``

  deb http://download.proxmox.com/debian/pbs bookworm pbstest

.. _package_repositories_client_only:

Proxmox Backup Client-only Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

If you want to :ref:`use the Proxmox Backup Client <client_creating_backups>`
on systems using a Linux distribution not based on Proxmox projects, you can
use the client-only repository.

Currently there's only a client-repository for APT based systems.

.. _package_repositories_client_only_apt:

APT-based Proxmox Backup Client Repository
++++++++++++++++++++++++++++++++++++++++++

For modern Linux distributions using `apt` as package manager, like all Debian
and Ubuntu Derivative do, you may be able to use the APT-based repository.

In order to configure this repository you need to first :ref:`setup the Proxmox
release key <package_repos_secure_apt>`. After that, add the repository URL to
the APT sources lists.

**Repositories for Debian 12 (Bookworm) based releases**

This repository is tested with:

- Debian Bookworm

Edit the file ``/etc/apt/sources.list.d/pbs-client.list`` and add the following
snippet

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://download.proxmox.com/debian/pbs-client bookworm main

**Repositories for Debian 11 (Bullseye) based releases**

This repository is tested with:

- Debian Bullseye

Edit the file ``/etc/apt/sources.list.d/pbs-client.list`` and add the following
snippet

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://download.proxmox.com/debian/pbs-client bullseye main

**Repositories for Debian 10 (Buster) based releases**

This repository is tested with:

- Debian Buster
- Ubuntu 20.04 LTS

It may work with older, and should work with more recent released versions.

Edit the file ``/etc/apt/sources.list.d/pbs-client.list`` and add the following
snippet

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://download.proxmox.com/debian/pbs-client buster main

.. _node_options_http_proxy:

Repository Access Behind HTTP Proxy
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Some setups have restricted access to the internet, sometimes only through a
central proxy. You can setup a HTTP proxy through the Proxmox Backup Server's
web-interface in the `Configuration -> Authentication` tab.

Once configured this proxy will be used for apt network requests and for
checking a Proxmox Backup Server support subscription.

Standard HTTP proxy configurations are accepted, `[http://]<host>[:port]` where
the `<host>` part may include an authorization, for example:
`http://user:pass@proxy.example.org:12345`

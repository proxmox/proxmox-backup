.. _sysadmin_package_repositories:

Debian Package Repositories
---------------------------

All Debian based systems use APT_ as package management tool. The list of
repositories is defined in ``/etc/apt/sources.list`` and ``.list`` files found
in the ``/etc/apt/sources.d/`` directory. Updates can be installed directly
with the ``apt`` command line tool, or via the GUI.

APT_ ``sources.list`` files list one package repository per line, with the most
preferred source listed first. Empty lines are ignored and a ``#`` character
anywhere on a line marks the remainder of that line as a comment. The
information available from the configured sources is acquired by ``apt
update``.

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list``

  deb http://ftp.debian.org/debian buster main contrib
  deb http://ftp.debian.org/debian buster-updates main contrib

  # security updates
  deb http://security.debian.org/debian-security buster/updates main contrib


.. FIXME for 7.0: change security update suite to bullseye-security

In addition, you need a package repositories from Proxmox to get the backup
server updates.

During the Proxmox Backup beta phase only one repository (pbstest) will be
available. Once released, a Enterprise repository for production use and a
no-subscription repository will be provided.

.. comment
    `Proxmox Backup`_ Enterprise Repository
    ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

    This will be the default, stable, and recommended repository. It is available for
    all `Proxmox Backup`_ subscription users. It contains the most stable packages,
    and is suitable for production use. The ``pbs-enterprise`` repository is
    enabled by default:

    .. note:: During the Proxmox Backup beta phase only one repository (pbstest)
     will be available.

    .. code-block:: sources.list
      :caption: File: ``/etc/apt/sources.list.d/pbs-enterprise.list``

      deb https://enterprise.proxmox.com/debian/pbs buster pbs-enterprise


    To never miss important security fixes, the superuser (``root@pam`` user) is
    notified via email about new packages as soon as they are available. The
    change-log and details of each package can be viewed in the GUI (if available).

    Please note that you need a valid subscription key to access this
    repository. More information regarding subscription levels and pricing can be
    found at https://www.proxmox.com/en/proxmox-backup/pricing.

    .. note:: You can disable this repository by commenting out the above
      line using a `#` (at the start of the line). This prevents error
      messages if you do not have a subscription key. Please configure the
      ``pbs-no-subscription`` repository in that case.


    `Proxmox Backup`_ No-Subscription Repository
    ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

    As the name suggests, you do not need a subscription key to access
    this repository. It can be used for testing and non-production
    use. It is not recommended to use it on production servers, because these
    packages are not always heavily tested and validated.

    We recommend to configure this repository in ``/etc/apt/sources.list``.

    .. code-block:: sources.list
      :caption: File: ``/etc/apt/sources.list``

      deb http://ftp.debian.org/debian buster main contrib
      deb http://ftp.debian.org/debian buster-updates main contrib

      # PBS pbs-no-subscription repository provided by proxmox.com,
      # NOT recommended for production use
      deb http://download.proxmox.com/debian/pbs buster pbs-no-subscription

      # security updates
      deb http://security.debian.org/debian-security buster/updates main contrib


`Proxmox Backup`_ Beta Repository
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

During the public beta, there is a repository called ``pbstest``. This one
contains the latest packages and is heavily used by developers to test new
features.

..  .. warning:: the ``pbstest`` repository should (as the name implies)
  only be used to test new features or bug fixes.

You can configure this using ``/etc/apt/sources.list`` by adding the following
line:

.. code-block:: sources.list
  :caption: sources.list entry for ``pbstest``

  deb http://download.proxmox.com/debian/pbs buster pbstest

If you installed Proxmox Backup Server from the official beta ISO you should
have this repository already configured in
``/etc/apt/sources.list.d/pbstest-beta.list``

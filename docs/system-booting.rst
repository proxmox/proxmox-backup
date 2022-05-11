
.. _chapter-systembooting:

Host Bootloader
---------------

`Proxmox Backup`_ currently uses one of two bootloaders depending on the disk setup
selected in the installer.

For EFI Systems installed with ZFS as the root filesystem ``systemd-boot`` is
used. All other deployments use the standard ``grub`` bootloader (this usually
also applies to systems which are installed on top of Debian).


.. _systembooting-installer-part-scheme:

Partitioning Scheme Used by the Installer
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The `Proxmox Backup`_ installer creates 3 partitions on all disks selected for
installation.

The created partitions are:

* a 1 MB BIOS Boot Partition (gdisk type EF02)

* a 512 MB EFI System Partition (ESP, gdisk type EF00)

* a third partition spanning the set ``hdsize`` parameter or the remaining space
  used for the chosen storage type

Systems using ZFS as root filesystem are booted with a kernel and initrd image
stored on the 512 MB EFI System Partition. For legacy BIOS systems, ``grub`` is
used, for EFI systems ``systemd-boot`` is used. Both are installed and configured
to point to the ESPs.

``grub`` in BIOS mode (``--target i386-pc``) is installed onto the BIOS Boot
Partition of all selected disks on all systems booted with ``grub`` (These are
all installs with root on ``ext4`` or ``xfs`` and installs with root on ZFS on
non-EFI systems).


.. _systembooting-proxmox-boot-tool:

Synchronizing the content of the ESP with ``proxmox-boot-tool``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``proxmox-boot-tool`` is a utility used to keep the contents of the EFI System
Partitions properly configured and synchronized. It copies certain kernel
versions to all ESPs and configures the respective bootloader to boot from
the ``vfat`` formatted ESPs. In the context of ZFS as root filesystem this means
that you can use all optional features on your root pool instead of the subset
which is also present in the ZFS implementation in ``grub`` or having to create a
separate small boot-pool (see: `Booting ZFS on root with grub
<https://github.com/zfsonlinux/zfs/wiki/Debian-Stretch-Root-on-ZFS>`_).

In setups with redundancy all disks are partitioned with an ESP, by the
installer. This ensures the system boots even if the first boot device fails
or if the BIOS can only boot from a particular disk.

The ESPs are not kept mounted during regular operation. This helps to prevent
filesystem corruption to the ``vfat`` formatted ESPs in case of a system crash,
and removes the need to manually adapt ``/etc/fstab`` in case the primary boot
device fails.

``proxmox-boot-tool`` handles the following tasks:

* formatting and setting up a new partition
* copying and configuring new kernel images and initrd images to all listed ESPs
* synchronizing the configuration on kernel upgrades and other maintenance tasks
* managing the list of kernel versions which are synchronized
* configuring the boot-loader to boot a particular kernel version (pinning)


You can view the currently configured ESPs and their state by running:

.. code-block:: console

  # proxmox-boot-tool status

.. _systembooting-proxmox-boot-setup:

Setting up a new partition for use as synced ESP
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To format and initialize a partition as synced ESP, e.g., after replacing a
failed vdev in an rpool, ``proxmox-boot-tool`` from ``pve-kernel-helper`` can be used.

WARNING: the ``format`` command will format the ``<partition>``, make sure to pass
in the right device/partition!

For example, to format an empty partition ``/dev/sda2`` as ESP, run the following:

.. code-block:: console

  # proxmox-boot-tool format /dev/sda2

To setup an existing, unmounted ESP located on ``/dev/sda2`` for inclusion in
`Proxmox Backup`_'s kernel update synchronization mechanism, use the following:

.. code-block:: console

  # proxmox-boot-tool init /dev/sda2

Afterwards `/etc/kernel/proxmox-boot-uuids`` should contain a new line with the
UUID of the newly added partition. The ``init`` command will also automatically
trigger a refresh of all configured ESPs.

.. _systembooting-proxmox-boot-refresh:

Updating the configuration on all ESPs
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To copy and configure all bootable kernels and keep all ESPs listed in
``/etc/kernel/proxmox-boot-uuids`` in sync you just need to run:

.. code-block:: console

  # proxmox-boot-tool refresh

(The equivalent to running ``update-grub`` systems with ``ext4`` or ``xfs`` on root).

This is necessary should you make changes to the kernel commandline, or want to
sync all kernels and initrds.

.. NOTE:: Both ``update-initramfs`` and ``apt`` (when necessary) will automatically
   trigger a refresh.

Kernel Versions considered by ``proxmox-boot-tool``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The following kernel versions are configured by default:

* the currently running kernel
* the version being newly installed on package updates
* the two latest already installed kernels
* the latest version of the second-to-last kernel series (e.g. 5.0, 5.3), if applicable
* any manually selected kernels

Manually keeping a kernel bootable
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Should you wish to add a certain kernel and initrd image to the list of
bootable kernels use ``proxmox-boot-tool kernel add``.

For example run the following to add the kernel with ABI version ``5.0.15-1-pve``
to the list of kernels to keep installed and synced to all ESPs:

.. code-block:: console

  # proxmox-boot-tool kernel add 5.0.15-1-pve


``proxmox-boot-tool kernel list`` will list all kernel versions currently selected
for booting:

.. code-block:: console

  # proxmox-boot-tool kernel list
  Manually selected kernels:
  5.0.15-1-pve

  Automatically selected kernels:
  5.0.12-1-pve
  4.15.18-18-pve

Run ``proxmox-boot-tool kernel remove`` to remove a kernel from the list of
manually selected kernels, for example:

.. code-block:: console

  # proxmox-boot-tool kernel remove 5.0.15-1-pve


.. NOTE:: It's required to run ``proxmox-boot-tool refresh`` to update all EFI System
   Partitions (ESPs) after a manual kernel addition or removal from above.


.. _systembooting-determine-bootloader:

Determine which Bootloader is Used
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~


The simplest and most reliable way to determine which bootloader is used, is to
watch the boot process of the `Proxmox Backup`_ node.

You will either see the blue box of ``grub`` or the simple black on white
``systemd-boot``.


Determining the bootloader from a running system might not be 100% accurate. The
safest way is to run the following command:


.. code-block:: console

  # efibootmgr -v


If it returns a message that EFI variables are not supported, ``grub`` is used in
BIOS/Legacy mode.

If the output contains a line that looks similar to the following, ``grub`` is
used in UEFI mode.

.. code-block:: console

  Boot0005* proxmox	[...] File(\EFI\proxmox\grubx64.efi)


If the output contains a line similar to the following, ``systemd-boot`` is used.

.. code-block:: console

  Boot0006* Linux Boot Manager	[...] File(\EFI\systemd\systemd-bootx64.efi)


By running:

.. code-block:: console

  # proxmox-boot-tool status


you can find out if ``proxmox-boot-tool`` is configured, which is a good
indication of how the system is booted.


.. _systembooting-grub:

Grub
~~~~

``grub`` has been the de-facto standard for booting Linux systems for many years
and is quite well documented
(see the `Grub Manual
<https://www.gnu.org/software/grub/manual/grub/grub.html>`_).

Configuration
^^^^^^^^^^^^^
Changes to the ``grub`` configuration are done via the defaults file
``/etc/default/grub`` or config snippets in ``/etc/default/grub.d``. To regenerate
the configuration file after a change to the configuration run:

.. code-block:: console

  # update-grub

.. NOTE:: Systems using ``proxmox-boot-tool`` will call
  ``proxmox-boot-tool refresh`` upon ``update-grub``

.. _systembooting-systemdboot:

Systemd-boot
~~~~~~~~~~~~

``systemd-boot`` is a lightweight EFI bootloader. It reads the kernel and initrd
images directly from the EFI Service Partition (ESP) where it is installed.
The main advantage of directly loading the kernel from the ESP is that it does
not need to reimplement the drivers for accessing the storage. In `Proxmox
Backup`_ :ref:`proxmox-boot-tool <systembooting-proxmox-boot-tool>` is used to
keep the configuration on the ESPs synchronized.

.. _systembooting-systemd-boot-config:

Configuration
^^^^^^^^^^^^^

``systemd-boot`` is configured via the file ``loader/loader.conf`` in the root
directory of an EFI System Partition (ESP). See the ``loader.conf(5)`` manpage
for details.

Each bootloader entry is placed in a file of its own in the directory
``loader/entries/``

An example entry.conf looks like this (``/`` refers to the root of the ESP):

.. code-block:: console

  title    Proxmox
  version  5.0.15-1-pve
  options   root=ZFS=rpool/ROOT/pve-1 boot=zfs
  linux    /EFI/proxmox/5.0.15-1-pve/vmlinuz-5.0.15-1-pve
  initrd   /EFI/proxmox/5.0.15-1-pve/initrd.img-5.0.15-1-pve


.. _systembooting-edit-kernel-cmdline:

Editing the Kernel Commandline
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You can modify the kernel commandline in the following places, depending on the
bootloader used:

Grub
^^^^

The kernel commandline needs to be placed in the variable
``GRUB_CMDLINE_LINUX_DEFAULT`` in the file ``/etc/default/grub``. Running
``update-grub`` appends its content to all ``linux`` entries in
``/boot/grub/grub.cfg``.

Systemd-boot
^^^^^^^^^^^^

The kernel commandline needs to be placed as one line in ``/etc/kernel/cmdline``.
To apply your changes, run ``proxmox-boot-tool refresh``, which sets it as the
``option`` line for all config files in ``loader/entries/proxmox-*.conf``.


.. _systembooting-kernel-pin:

Override the Kernel-Version for next Boot
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

To select a kernel that is not currently the default kernel, you can either:

* use the boot loader menu that is displayed at the beginning of the boot
  process
* use the ``proxmox-boot-tool`` to ``pin`` the system to a kernel version either
  once or permanently (until pin is reset).

This should help you work around incompatibilities between a newer kernel
version and the hardware.

.. NOTE:: Such a pin should be removed as soon as possible so that all current
   security patches of the latest kernel are also applied to the system.

For example: To permanently select the version ``5.15.30-1-pve`` for booting you
would run:

.. code-block:: console

  # proxmox-boot-tool kernel pin 5.15.30-1-pve


.. TIP:: The pinning functionality works for all `Proxmox Backup`_ systems, not only those using
   ``proxmox-boot-tool`` to synchronize the contents of the ESPs, if your system
   does not use ``proxmox-boot-tool`` for synchronizing you can also skip the
   ``proxmox-boot-tool refresh`` call in the end.

You can also set a kernel version to be booted on the next system boot only.
This is for example useful to test if an updated kernel has resolved an issue,
which caused you to ``pin`` a version in the first place:

.. code-block:: console

  # proxmox-boot-tool kernel pin 5.15.30-1-pve --next-boot


To remove any pinned version configuration use the ``unpin`` subcommand:

.. code-block:: console

  # proxmox-boot-tool kernel unpin

While ``unpin`` has a ``--next-boot`` option as well, it is used to clear a pinned
version set with ``--next-boot``. As that happens already automatically on boot,
invonking it manually is of little use.

After setting, or clearing pinned versions you also need to synchronize the
content and configuration on the ESPs by running the ``refresh`` subcommand.

.. TIP:: You will be prompted to automatically do for  ``proxmox-boot-tool`` managed
   systems if you call the tool interactively.

.. code-block:: console

  # proxmox-boot-tool refresh

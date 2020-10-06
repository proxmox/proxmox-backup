System Requirements
-------------------

We recommend using high quality server hardware when running Proxmox Backup in
production. To further decrease the impact of a failed host, you can set up
periodic, efficient, incremental :ref:`datastore synchronization <syncjobs>`
from other Proxmox Backup Server instances.

Minimum Server Requirements, for Evaluation
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

These minimum requirements are for evaluation purposes only and should not be
used in production.

* CPU: 64bit (*x86-64* or *AMD64*), 2+ Cores

* Memory (RAM): 2 GB RAM

* Hard drive: more than 8GB of space.

* Network card (NIC)


Recommended Server System Requirements
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* CPU: Modern AMD or Intel 64-bit based CPU, with at least 4 cores

* Memory: minimum 4 GiB for the OS, filesystem cache and Proxmox Backup Server
  daemons. Add at least another GiB per TiB storage space.

* OS storage:

  * 32 GiB, or more, free storage space
  * Use a hardware RAID with battery protected write cache (*BBU*) or a
    redundant ZFS setup (ZFS is not compatible with a hardware RAID
    controller).

* Backup storage:

  * Use only SSDs, for best results
  * If HDDs are used: Using a metadata cache is highly recommended, for example,
    add a ZFS :ref:`special device mirror <local_zfs_special_device>`.

* Redundant Multi-GBit/s network interface cards (NICs)


Supported Web Browsers for Accessing the Web Interface
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

To access the server's web-based user interface, we recommend using one of the
following browsers:

* Firefox, a release from the current year, or the latest Extended Support Release
* Chrome, a release from the current year
* Microsoft's currently supported version of Edge
* Safari, a release from the current year

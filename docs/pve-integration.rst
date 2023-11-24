.. _pve-integration:

`Proxmox VE`_ Integration
-------------------------

`Proxmox Backup`_ Server can be integrated into a Proxmox VE standalone or
cluster setup, by adding it as a storage in Proxmox VE.

See also the `Proxmox VE Storage - Proxmox Backup Server
<https://pve.proxmox.com/pve-docs/pve-admin-guide.html#storage_pbs>`_ section
of the Proxmox VE Administration Guide for Proxmox VE specific documentation.


Using the Proxmox VE Web-Interface
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Proxmox VE has native API and web interface integration of Proxmox Backup
Server as of `Proxmox VE 6.3
<https://pve.proxmox.com/wiki/Roadmap#Proxmox_VE_6.3>`_.

A Proxmox Backup Server can be added under ``Datacenter -> Storage``.

Using the Proxmox VE Command Line
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You need to define a new storage with type 'pbs' on your `Proxmox VE`_
node. The following example uses ``store2`` as the storage's name, and
assumes the server address is ``localhost`` and you want to connect
as ``user1@pbs``.

.. code-block:: console

  # pvesm add pbs store2 --server localhost --datastore store2
  # pvesm set store2 --username user1@pbs --password <secret>

.. note:: If you would rather not enter your password as plain text, you can pass
  the ``--password`` parameter, without any arguments. This will cause the
  program to prompt you for a password upon entering the command.

If your backup server uses a self signed certificate, you need to add
the certificate fingerprint to the configuration. You can get the
fingerprint by running the following command on the backup server:

.. code-block:: console

  # proxmox-backup-manager cert info | grep Fingerprint
  Fingerprint (sha256): 64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

Please add that fingerprint to your configuration to establish a trust
relationship:

.. code-block:: console

  # pvesm set store2 --fingerprint  64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

After that, you should be able to view storage status with:

.. code-block:: console

  # pvesm status --storage store2
  Name             Type     Status           Total            Used       Available        %
  store2            pbs     active      3905109820      1336687816      2568422004   34.23%

Having added the Proxmox Backup Server datastore to `Proxmox VE`_, you can
backup VMs and containers in the same way you would for any other storage
device within the environment (see `Proxmox VE Admin Guide: Backup and Restore
<https://pve.proxmox.com/pve-docs/pve-admin-guide.html#chapter_vzdump>`_.



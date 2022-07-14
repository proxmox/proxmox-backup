==========================
proxmox-backup
==========================

Synopsis
==========

This daemon is normally started and managed as ``systemd`` service::

 systemctl start proxmox-backup

 systemctl stop proxmox-backup

 systemctl status proxmox-backup

For debugging, you can start the daemon in foreground using::

 proxmox-backup-api

.. NOTE:: You need to stop the service before starting the daemon in
   foreground.

Description
============

.. include:: description.rst

.. include:: ../pbs-copyright.rst

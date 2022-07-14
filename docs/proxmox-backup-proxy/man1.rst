==========================
proxmox-backup-proxy
==========================

Synopsis
==========

This daemon is normally started and managed as ``systemd`` service::

 systemctl start proxmox-backup-proxy

 systemctl stop proxmox-backup-proxy

 systemctl status proxmox-backup-proxy

For debugging, you can start the daemon in foreground using::

 proxmox-backup-proxy

.. NOTE:: You need to stop the service before starting the daemon in
   foreground.

Description
============

.. include:: description.rst

.. include:: ../pbs-copyright.rst

Introduction
============

This documentationm is written in :term:`reStructuredText` and formatted with :term:`Sphinx`.

What is Proxmox Backup
----------------------

Proxmox Backup is an enterprise class client-server backup software,
specially optimized for `Proxmox Virtual Environment`_ to backup
:term:`virtual machine`\ s and :term:`container`\ s.


Main features
-------------

:Proxmox VE: The `Proxmox Virtual Environment`_ is fully
   supported. You can backup :term:`virtual machine`\ s and
   :term:`container`\ s.

:GUI: We provide a graphical, web based user interface.

:Deduplication: Inkremental backup produces large amounts of duplicate
   data. The deduplication layer removes that redundancy and makes
   inkremental backup small and space efficient.

:Data Integrity: The built in `SHA-256`_ checksum algorithm assures the
   accuray and consistency of your backups.

:Remote Sync: It is possible to efficently synchronize data to remote
   sites. Only deltas containing new data are transfered.

:Performance: The whole software stack is written in :term:`Rust`,
   which provides high speed and memory efficiency.

:Compression: Ultra fast `LZ4`_ compression is able to compress
   several gigabytes of data per second.

:Open Source: No secrets. You have access to the whole source tree.

:Support: Commercial support options available from `Proxmox`_.

Why Backup?
-----------

The primary purpose of backup is to protect against data loss. Data
loss can happen because of faulty hardware, but also by human errors.

A common mistake is to delete a file or folder which is still
required. Virtualization can amplify this problem, because it is now
easy to delete a whole virtual machine by a single button press.

Backups can also serve as a toolkit for administrators to temporarily
store data. For example, it is common practice to perform full backups
before installing major software updates. If something goes wrong, you
can just restore the previous state.

Another reason for backups are legal requirements. Some data must be
kept in a safe place for several years so that you can access it if
required by law.


- value of data, importance for your business, legal regulations

- restore tests. to make sure backup/restore works




History
-------

history ...

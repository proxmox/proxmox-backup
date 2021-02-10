==========================
datastore.cfg
==========================

.. include:: ../../epilog.rst

-------------------------------------------------------------
Datastore Configuration
-------------------------------------------------------------

:Author: |AUTHOR|
:Version: Version |VERSION|
:Manual section: 5

Description
===========

The file /etc/proxmox-backup/datastore.cfg is a configuration file for Proxmox
Backup Server. It contains the Datastore configuration.

File Format
===========

The file contains a list of datastore configuration sections. Each
section starts with a header ``datastore: <name>``, followed by the
datastore configuration options.

::
  
  datastore: <name1>
     path <path1>
     <option1> <value1>
     ...

  datastore: <name2>
     path <path2>
     ...

     
Options
=======

.. include:: config.rst


.. include:: ../../pbs-copyright.rst

Ext.define('PBS.StorageAndDiskPanel', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsStorageAndDiskPanel',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext('Storage / Disks'),

    tools: [PBS.Utils.get_help_tool("storage-disk-management")],

    border: false,
    defaults: {
	border: false,
    },

    items: [
	{
	    xtype: 'pmxDiskList',
	    title: gettext('Disks'),
	    includePartitions: true,
	    supportsWipeDisk: true,
	    itemId: 'disks',
	    iconCls: 'fa fa-hdd-o',
	},
	{
	    xtype: 'pbsDirectoryList',
	    title: Proxmox.Utils.directoryText,
	    itemId: 'directorystorage',
	    iconCls: 'fa fa-folder',
	},
	{
	    xtype: 'pbsZFSList',
	    title: "ZFS",
	    nodename: 'localhost',
	    iconCls: 'fa fa-th-large',
	    itemId: 'zfsstorage',
	},
    ],

});

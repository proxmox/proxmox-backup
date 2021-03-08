Ext.define('PBS.TapeManagement', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsTapeManagement',

    title: gettext('Tape Backup'),

    tools: [PBS.Utils.get_help_tool("tape_backup")],

    border: true,
    defaults: {
	border: false,
	xtype: 'panel',
    },

    items: [
	{
	    xtype: 'pbsBackupOverview',
	    title: gettext('Content'),
	    itemId: 'content',
	    iconCls: 'fa fa-th',
	},
	{
	    xtype: 'pbsTapeInventory',
	    title: gettext('Inventory'),
	    itemId: 'inventory',
	    iconCls: 'fa fa-book',
	},
	{
	    xtype: 'pbsTapeChangerPanel',
	    title: gettext('Changers'),
	    itemId: 'changers',
	    iconCls: 'fa fa-exchange',
	},
	{
	    xtype: 'pbsTapeDrivePanel',
	    title: gettext('Drives'),
	    itemId: 'drives',
	    //iconCls: 'fa fa-download',
	    iconCls: 'pbs-icon-tape-drive',
	},
	{
	    title: gettext('Media Pools'),
	    itemId: 'pools',
	    xtype: 'pbsMediaPoolPanel',
	    iconCls: 'fa fa-object-group',
	},
	{
	    xtype: 'pbsEncryptionKeys',
	    title: gettext('Encryption Keys'),
	    itemId: 'encryption-keys',
	    iconCls: 'fa fa-key',
	},
	{
	    xtype: 'pbsTapeBackupJobView',
	    title: gettext('Backup Jobs'),
	    itemId: 'tape-backup-jobs',
	    iconCls: 'fa fa-floppy-o',
	},
    ],
});

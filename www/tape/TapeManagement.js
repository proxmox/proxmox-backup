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
	    title: gettext('Content'),
	    itemId: 'content',
	    xtype: 'pbsBackupOverview',
	},
	{
	    title: gettext('Inventory'),
	    itemId: 'inventory',
	    xtype: 'pbsTapeInventory',
	},
	{
	    title: gettext('Changers'),
	    itemId: 'changers',
	    xtype: 'pbsTapeChangerPanel',
	},
	{
	    title: gettext('Drives'),
	    itemId: 'drives',
	    xtype: 'pbsTapeDrivePanel',
	},
	{
	    title: gettext('Media Pools'),
	    itemId: 'pools',
	    xtype: 'pbsMediaPoolPanel',
	},
	{
	    title: gettext('Encryption Keys'),
	    itemId: 'encryption-keys',
	    xtype: 'pbsEncryptionKeys',
	},
	{
	    title: gettext('Backup Jobs'),
	    itemId: 'tape-backup-jobs',
	    xtype: 'pbsTapeBackupJobView',
	},
    ],
});

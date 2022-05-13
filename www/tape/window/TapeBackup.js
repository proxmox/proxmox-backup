Ext.define('PBS.TapeManagement.TapeBackupWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'pbsTapeBackupWindow',

    subject: gettext('Backup'),
    url: '/api2/extjs/tape/backup',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

    items: [
	{
	    xtype: 'inputpanel',
	    column1: [
		{
		    xtype: 'pbsDataStoreSelector',
		    fieldLabel: gettext('Datastore'),
		    name: 'store',
		    listeners: {
			change: function(_, value) {
			    let me = this;
			    if (value) {
				let namespace = me.up('window').down('pbsNamespaceSelector');
				namespace.setDatastore(value);
				namespace.setDisabled(false);
				me.up('window').down('pbsNamespaceMaxDepth').setDisabled(false);
			    }
			},
		    },
		},
		{
		    xtype: 'pbsNamespaceSelector',
		    fieldLabel: gettext('Namespace'),
		    disabled: true,
		    name: 'ns',
		},
		{
		    xtype: 'pbsNamespaceMaxDepth',
		    fieldLabel: gettext('Max Depth'),
		    disabled: true,
		    name: 'max-depth',
		},
		{
		    xtype: 'pbsMediaPoolSelector',
		    fieldLabel: gettext('Media Pool'),
		    name: 'pool',
		},
		{
		    xtype: 'pbsDriveSelector',
		    fieldLabel: gettext('Drive'),
		    name: 'drive',
		},
	    ],

	    column2: [
		{
		    xtype: 'proxmoxcheckbox',
		    labelWidth: 150,
		    name: 'force-media-set',
		    fieldLabel: gettext('Force new Media-Set'),
		},
		{
		    xtype: 'proxmoxcheckbox',
		    labelWidth: 150,
		    name: 'export-media-set',
		    fieldLabel: gettext('Export Media-Set'),
		    listeners: {
			change: function(cb, value) {
			    let me = this;
			    let eject = me.up('window').down('proxmoxcheckbox[name=eject-media]');
			    if (value) {
				eject.setValue(false);
			    }
			    eject.setDisabled(!!value);
			},
		    },
		},
		{
		    xtype: 'proxmoxcheckbox',
		    labelWidth: 150,
		    name: 'eject-media',
		    fieldLabel: gettext('Eject Media'),
		},
		{
		    xtype: 'pmxUserSelector',
		    labelWidth: 150,
		    name: 'notify-user',
		    fieldLabel: gettext('Notify User'),
		    emptyText: 'root@pam',
		    value: null,
		    allowBlank: true,
		    skipEmptyText: true,
		    renderer: Ext.String.htmlEncode,
		},
	    ],
	},
    ],

});

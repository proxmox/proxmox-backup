Ext.define('PBS.TapeManagement.TapeBackupWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'pbsTapeBackupWindow',

    subject: gettext('Backup'),
    url: '/api2/extjs/tape/backup',
    method: 'POST',
    showTaskViewer: true,
    isCreate: true,

    viewModel: {
	data: {
	    notificationMode: 'notification-system',
	},
	formulas: {
	    notificationSystemSelected: (get) => get('notificationMode') === 'notification-system',
	},
    },

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
		    deleteEmpty: false,
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
		    xtype: 'proxmoxKVComboBox',
		    labelWidth: 150,
		    comboItems: [
			['legacy-sendmail', gettext('Email (legacy)')],
			['notification-system', gettext('Notification system')],
		    ],
		    fieldLabel: gettext('Notification mode'),
		    name: 'notification-mode',
		    bind: {
			value: '{notificationMode}',
		    },
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
		    bind: {
			disabled: "{notificationSystemSelected}",
		    },
		    renderer: Ext.String.htmlEncode,
		},
	    ],
	},
    ],

});

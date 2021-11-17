Ext.define('PBS.TapeManagement.BackupJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTapeBackupJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    isAdd: true,

    subject: gettext('Tape Backup Job'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/tape-backup-job';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	me.scheduleValue = id ? null : 'daily';
	me.authid = id ? null : Proxmox.UserName;
	me.editDatastore = me.datastore === undefined && me.isCreate;
	return { };
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (values['export-media-set'] && !me.up('pbsTapeBackupJobEdit').isCreate) {
		Proxmox.Utils.assemble_field_data(values, { "delete": 'eject-media' });
	    }
	    PBS.Utils.delete_if_default(values, 'notify-user');
	    return values;
	},
	column1: [
	    {
		xtype: 'pmxDisplayEditField',
		name: 'id',
		fieldLabel: gettext('Job ID'),
		renderer: Ext.htmlEncode,
		allowBlank: false,
		cbind: {
		    editable: '{isCreate}',
		},
	    },
	    {
		xtype: 'pbsDataStoreSelector',
		fieldLabel: gettext('Local Datastore'),
		name: 'store',
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
	    {
		xtype: 'pmxUserSelector',
		name: 'notify-user',
		fieldLabel: gettext('Notify User'),
		emptyText: 'root@pam',
		allowBlank: true,
		value: null,
		renderer: Ext.String.htmlEncode,
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Schedule'),
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		emptyText: gettext('none (disabled)'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		    value: '{scheduleValue}',
		},
	    },
	    {
		fieldLabel: gettext('Export Media-Set'),
		xtype: 'proxmoxcheckbox',
		name: 'export-media-set',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
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
		fieldLabel: gettext('Eject Media'),
		xtype: 'proxmoxcheckbox',
		name: 'eject-media',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Latest Only'),
		xtype: 'proxmoxcheckbox',
		name: 'latest-only',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],

	columnB: [
	    {
		fieldLabel: gettext('Comment'),
		xtype: 'proxmoxtextfield',
		name: 'comment',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],
    },
});

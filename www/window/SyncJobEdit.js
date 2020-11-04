Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'syncjobs',

    isAdd: true,

    subject: gettext('SyncJob'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/sync';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	return { };
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (!values.id && me.up('pbsSyncJobEdit').isCreate) {
		values.id = 'auto-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 23);
	    }
	    return values;
	},
	column1: [
	    {
		xtype: 'displayfield',
		name: 'store',
		fieldLabel: gettext('Local Datastore'),
		allowBlank: false,
		submitValue: true,
		cbind: {
		    value: '{datastore}',
		},
	    },
	    {
		fieldLabel: gettext('Source Remote'),
		xtype: 'pbsRemoteSelector',
		allowBlank: false,
		name: 'remote',
	    },
	    {
		fieldLabel: gettext('Source Datastore'),
		xtype: 'proxmoxtextfield',
		allowBlank: false,
		name: 'remote-store',
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Owner'),
		xtype: 'pbsUserSelector',
		name: 'owner',
		allowBlank: true,
		emptyText: 'backup@pam',
		skipEmptyText: true,
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Remove vanished'),
		xtype: 'proxmoxcheckbox',
		name: 'remove-vanished',
		uncheckedValue: false,
		value: false,
	    },
	    {
		fieldLabel: gettext('Schedule'),
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		value: 'hourly',
		emptyText: gettext('none (disabled)'),
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

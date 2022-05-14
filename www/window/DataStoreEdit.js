Ext.define('PBS.DataStoreEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsDataStoreEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    subject: gettext('Datastore'),
    isAdd: true,

    bodyPadding: 0,
    showProgress: true,

    cbindData: function(initialConfig) {
	var me = this;

	let name = initialConfig.name;
	let baseurl = '/api2/extjs/config/datastore';

	me.isCreate = !name;
	if (!me.isCreate) {
	    me.defaultFocus = 'textfield[name=comment]';
	}
	me.url = name ? baseurl + '/' + name : baseurl;
	me.method = name ? 'PUT' : 'POST';
	me.scheduleValue = name ? null : 'daily';
	me.autoLoad = !!name;
	return {};
    },

    items: {
	xtype: 'tabpanel',
	bodyPadding: 10,
	listeners: {
	    tabchange: function(tb, newCard) {
	        Ext.GlobalEvents.fireEvent('proxmoxShowHelp', newCard.onlineHelp);
	    },
	},
	items: [
	    {
		title: gettext('General'),
		xtype: 'inputpanel',
		onlineHelp: 'datastore_intro',
		column1: [
		    {
			xtype: 'pmxDisplayEditField',
			cbind: {
			    editable: '{isCreate}',
			},
			name: 'name',
			allowBlank: false,
			fieldLabel: gettext('Name'),
		    },
		    {
			xtype: 'pmxDisplayEditField',
			cbind: {
			    editable: '{isCreate}',
			},
			name: 'path',
			allowBlank: false,
			fieldLabel: gettext('Backing Path'),
			emptyText: gettext('An absolute path'),
		    },
		],
		column2: [
		    {
			xtype: 'pbsCalendarEvent',
			name: 'gc-schedule',
			fieldLabel: gettext("GC Schedule"),
			emptyText: gettext('none'),
			cbind: {
			    deleteEmpty: '{!isCreate}',
			    value: '{scheduleValue}',
			},
		    },
		    {
			xtype: 'pbsCalendarEvent',
			name: 'prune-schedule',
			fieldLabel: gettext("Prune Schedule"),
			value: 'daily',
			emptyText: gettext('none'),
			cbind: {
			    deleteEmpty: '{!isCreate}',
			    value: '{scheduleValue}',
			},
		    },
		],
		columnB: [
		    {
			xtype: 'textfield',
			name: 'comment',
			fieldLabel: gettext('Comment'),
		    },
		],
	    },
	    {
		title: gettext('Prune Options'),
		xtype: 'pbsPruneInputPanel',
		cbind: {
		    isCreate: '{isCreate}',
		},
		onlineHelp: 'backup_pruning',
	    },
	],
    },
});

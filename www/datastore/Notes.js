Ext.define('PBS.DataStoreNotes', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsDataStoreNotes',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext("Comment"),
    bodyStyle: 'white-space:pre',
    bodyPadding: 10,
    scrollable: true,
    animCollapse: false,

    cbindData: function(initalConfig) {
	let me = this;
	me.url = `/api2/extjs/config/datastore/${me.datastore}`;
	return { };
    },

    run_editor: function() {
	let me = this;
	let win = Ext.create('Proxmox.window.Edit', {
	    title: gettext('Comment'),
	    width: 600,
	    resizable: true,
	    layout: 'fit',
	    defaultButton: undefined,
	    items: {
		xtype: 'textfield',
		name: 'comment',
		value: '',
		hideLabel: true,
	    },
	    url: me.url,
	    listeners: {
		destroy: function() {
		    me.load();
		},
	    },
	}).show();
	win.load();
    },

    setNotes: function(value) {
	let me = this;
	var data = value || '';
	me.update(Ext.htmlEncode(data));

	if (me.collapsible && me.collapseMode === 'auto') {
	    me.setCollapsed(data === '');
	}
    },

    load: function() {
	var me = this;

	Proxmox.Utils.API2Request({
	    url: me.url,
	    waitMsgTarget: me,
	    failure: function(response, opts) {
		Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		me.setCollapsed(false);
	    },
	    success: function(response, opts) {
		me.setNotes(response.result.data.comment);
	    },
	});
    },

    listeners: {
	render: function(c) {
	    var me = this;
	    me.getEl().on('dblclick', me.run_editor, me);
	},
	afterlayout: function() {
	    let me = this;
	    if (me.collapsible && !me.getCollapsed() && me.collapseMode === 'always') {
		me.setCollapsed(true);
		me.collapseMode = ''; // only once, on initial load!
	    }
	},
    },

    tools: [{
	type: 'gear',
	handler: function() {
	    this.up('panel').run_editor();
	},
    }],

    collapsible: true,
    collapseDirection: 'right',

    initComponent: function() {
	var me = this;

	me.callParent();

	let sp = Ext.state.Manager.getProvider();
	me.collapseMode = sp.get('notes-collapse', 'never');

	if (me.collapseMode === 'auto') {
	    me.setCollapsed(true);
	}
    },
});

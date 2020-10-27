Ext.define('PBS.SubscriptionKeyEdit', {
    extend: 'Proxmox.window.Edit',

    title: gettext('Upload Subscription Key'),
    width: 300,
    autoLoad: true,

    onlineHelp: 'getting_help',

    items: {
	xtype: 'textfield',
	name: 'key',
	value: '',
	fieldLabel: gettext('Subscription Key'),
    },
});

Ext.define('PBS.Subscription', {
    extend: 'Proxmox.grid.ObjectGrid',
    xtype: 'pbsSubscription',

    title: gettext('Subscription'),
    border: true,

    onlineHelp: 'getting_help',

    viewConfig: {
	enableTextSelection: true,
    },

    initComponent: function() {
	let me = this;

	let reload = () => me.rstore.load();
	let baseurl = '/nodes/localhost/subscription';

	let rows = {
	    productname: {
		header: gettext('Type'),
	    },
	    key: {
		header: gettext('Subscription Key'),
	    },
	    status: {
		header: gettext('Status'),
		renderer: (value) => {
		    value = Ext.String.capitalize(value);
		    let message = me.getObjectValue('message');
		    if (message) {
			return value + ": " + message;
		    }
		    return value;
		},
	    },
	    message: {
		visible: false,
	    },
	    serverid: {
		header: gettext('Server ID'),
	    },
	    checktime: {
		header: gettext('Last checked'),
		renderer: Proxmox.Utils.render_timestamp,
	    },
	    nextduedate: {
		header: gettext('Next due date'),
	    },
	};

	Ext.apply(me, {
	    url: `/api2/json${baseurl}`,
	    cwidth1: 170,
	    tbar: [
		{
		    text: gettext('Upload Subscription Key'),
		    handler: function() {
			let win = Ext.create('PBS.SubscriptionKeyEdit', {
			    url: '/api2/extjs/' + baseurl,
			});
			win.show();
			win.on('destroy', reload);
		    },
		},
		{
		    text: gettext('Check'),
		    handler: function() {
			Proxmox.Utils.API2Request({
			    params: { force: 1 },
			    url: baseurl,
			    method: 'POST',
			    waitMsgTarget: me,
			    failure: function(response, opts) {
				Ext.Msg.alert(gettext('Error'), response.htmlStatus);
			    },
			    callback: reload,
			});
		    },
		},
	    ],
	    rows: rows,
	});

	me.callParent();

	reload();
    },
});

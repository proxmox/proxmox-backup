Ext.define('PBS.panel.XtermJsConsole', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsXtermJsConsole',

    layout: 'fit',

    items: [
	{
	    xtype: 'uxiframe',
	    itemId: 'iframe',
	},
    ],

    listeners: {
	'afterrender': function() {
	    let me = this;
	    let params = {
		console: 'shell',
		node: 'localhost',
		xtermjs: 1,
	    };
	    me.getComponent('iframe').load('/?' + Ext.Object.toQueryString(params));
	},
    },
});
